-- Forward-only repair for defaults that became unusable outside the model
-- profile Store contract. Historical profiles, versions, conversations,
-- settings, and keyring references remain intact.
UPDATE ai_user_model_defaults
SET default_conversation_profile_id = CASE
        WHEN default_conversation_profile_id IS NULL
          OR EXISTS (
              SELECT 1
              FROM ai_model_profiles p
              JOIN ai_model_profile_versions v
                ON v.profile_id = p.id AND v.version = p.current_version
              WHERE p.id = ai_user_model_defaults.default_conversation_profile_id
                AND p.user_id = ai_user_model_defaults.user_id
                AND p.archived_at IS NULL
                AND p.deleted_at IS NULL
          )
        THEN default_conversation_profile_id
        ELSE NULL
    END,
    default_vision_profile_id = CASE
        WHEN default_vision_profile_id IS NULL
          OR EXISTS (
              SELECT 1
              FROM ai_model_profiles p
              JOIN ai_model_profile_versions v
                ON v.profile_id = p.id AND v.version = p.current_version
              WHERE p.id = ai_user_model_defaults.default_vision_profile_id
                AND p.user_id = ai_user_model_defaults.user_id
                AND p.archived_at IS NULL
                AND p.deleted_at IS NULL
                AND v.supports_vision = 1
          )
        THEN default_vision_profile_id
        ELSE NULL
    END,
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    revision = revision + 1
WHERE deleted_at IS NULL
  AND (
      (
          default_conversation_profile_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM ai_model_profiles p
              JOIN ai_model_profile_versions v
                ON v.profile_id = p.id AND v.version = p.current_version
              WHERE p.id = ai_user_model_defaults.default_conversation_profile_id
                AND p.user_id = ai_user_model_defaults.user_id
                AND p.archived_at IS NULL
                AND p.deleted_at IS NULL
          )
      )
      OR (
          default_vision_profile_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM ai_model_profiles p
              JOIN ai_model_profile_versions v
                ON v.profile_id = p.id AND v.version = p.current_version
              WHERE p.id = ai_user_model_defaults.default_vision_profile_id
                AND p.user_id = ai_user_model_defaults.user_id
                AND p.archived_at IS NULL
                AND p.deleted_at IS NULL
                AND v.supports_vision = 1
          )
      )
  );
