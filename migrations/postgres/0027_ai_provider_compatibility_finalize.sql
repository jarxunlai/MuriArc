-- Forward-only repair for defaults that became unusable outside the model
-- profile Store contract. Historical profiles, versions, conversations,
-- settings, and encrypted profile-version credentials remain intact.
UPDATE ai_user_model_defaults AS defaults
SET default_conversation_profile_id = CASE
        WHEN defaults.default_conversation_profile_id IS NULL
          OR EXISTS (
              SELECT 1
              FROM ai_model_profiles profile
              JOIN ai_model_profile_versions version
                ON version.profile_id = profile.id
               AND version.version = profile.current_version
              WHERE profile.id = defaults.default_conversation_profile_id
                AND profile.user_id = defaults.user_id
                AND profile.archived_at IS NULL
                AND profile.deleted_at IS NULL
          )
        THEN defaults.default_conversation_profile_id
        ELSE NULL
    END,
    default_vision_profile_id = CASE
        WHEN defaults.default_vision_profile_id IS NULL
          OR EXISTS (
              SELECT 1
              FROM ai_model_profiles profile
              JOIN ai_model_profile_versions version
                ON version.profile_id = profile.id
               AND version.version = profile.current_version
              WHERE profile.id = defaults.default_vision_profile_id
                AND profile.user_id = defaults.user_id
                AND profile.archived_at IS NULL
                AND profile.deleted_at IS NULL
                AND version.supports_vision
          )
        THEN defaults.default_vision_profile_id
        ELSE NULL
    END,
    updated_at = now(),
    revision = defaults.revision + 1
WHERE defaults.deleted_at IS NULL
  AND (
      (
          defaults.default_conversation_profile_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM ai_model_profiles profile
              JOIN ai_model_profile_versions version
                ON version.profile_id = profile.id
               AND version.version = profile.current_version
              WHERE profile.id = defaults.default_conversation_profile_id
                AND profile.user_id = defaults.user_id
                AND profile.archived_at IS NULL
                AND profile.deleted_at IS NULL
          )
      )
      OR (
          defaults.default_vision_profile_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM ai_model_profiles profile
              JOIN ai_model_profile_versions version
                ON version.profile_id = profile.id
               AND version.version = profile.current_version
              WHERE profile.id = defaults.default_vision_profile_id
                AND profile.user_id = defaults.user_id
                AND profile.archived_at IS NULL
                AND profile.deleted_at IS NULL
                AND version.supports_vision
          )
      )
  );
