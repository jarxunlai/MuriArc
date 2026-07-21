DO $$
BEGIN
    IF EXISTS (
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
    ) THEN
        RAISE EXCEPTION 'cannot constrain experiments: invalid template version references exist';
    END IF;
END
$$;

ALTER TABLE experiment_template_versions
    ADD CONSTRAINT experiment_template_versions_lab_id_id_unique
    UNIQUE (lab_id, id);

ALTER TABLE experiments
    ADD CONSTRAINT experiments_template_version_lab_fk
    FOREIGN KEY (lab_id, template_version_id)
    REFERENCES experiment_template_versions (lab_id, id)
    ON UPDATE RESTRICT
    ON DELETE RESTRICT;

CREATE FUNCTION muriarc_require_published_experiment_template()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.template_version_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
        FROM experiment_template_versions AS t
        WHERE t.id = NEW.template_version_id
          AND t.lab_id = NEW.lab_id
          AND t.status = 'published'
          AND t.deleted_at IS NULL
    ) THEN
        RAISE EXCEPTION 'experiment template must be an active published version in the same lab'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER trg_experiments_require_published_template
BEFORE INSERT OR UPDATE OF template_version_id, lab_id ON experiments
FOR EACH ROW
EXECUTE FUNCTION muriarc_require_published_experiment_template();
