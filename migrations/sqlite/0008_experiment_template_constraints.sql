-- Fail closed if a pre-existing experiment points at a missing, cross-lab,
-- deleted, or never-published template version. Retired versions remain valid
-- historical references. The guard table is temporary to
-- this migration and its CHECK constraint turns any conflict into a migration
-- failure instead of silently accepting inconsistent historical data.
CREATE TABLE _muriarc_experiment_template_guard (
    invalid INTEGER NOT NULL CHECK (invalid = 0)
);

INSERT INTO _muriarc_experiment_template_guard (invalid)
SELECT 1
WHERE EXISTS (
    SELECT 1
    FROM experiments AS e
    LEFT JOIN experiment_template_versions AS t
        ON t.id = e.template_version_id
    WHERE e.template_version_id IS NOT NULL
      AND (
          t.id IS NULL
          OR t.lab_id <> e.lab_id
          OR t.status NOT IN ('published', 'retired')
          OR t.deleted_at IS NOT NULL
      )
);

DROP TABLE _muriarc_experiment_template_guard;

-- SQLite cannot add a foreign key to an existing table without rebuilding it.
-- These triggers provide the same active-reference and same-lab protection
-- while also enforcing the domain rule that only published versions may be
-- selected for a new experiment.
CREATE TRIGGER trg_experiments_template_insert
BEFORE INSERT ON experiments
FOR EACH ROW
WHEN NEW.template_version_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM experiment_template_versions AS t
     WHERE t.id = NEW.template_version_id
       AND t.lab_id = NEW.lab_id
       AND t.status = 'published'
       AND t.deleted_at IS NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'experiment template must be an active published version in the same lab');
END;

CREATE TRIGGER trg_experiments_template_update
BEFORE UPDATE OF template_version_id, lab_id ON experiments
FOR EACH ROW
WHEN NEW.template_version_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM experiment_template_versions AS t
     WHERE t.id = NEW.template_version_id
       AND t.lab_id = NEW.lab_id
       AND t.status = 'published'
       AND t.deleted_at IS NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'experiment template must be an active published version in the same lab');
END;

CREATE TRIGGER trg_template_delete_restrict
BEFORE DELETE ON experiment_template_versions
FOR EACH ROW
WHEN EXISTS (
    SELECT 1
    FROM experiments AS e
    WHERE e.template_version_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'experiment template is referenced by an experiment');
END;
