ALTER TABLE ai_conversation_messages
    ADD COLUMN source_refs_json JSONB NOT NULL DEFAULT '[]'::jsonb
    CHECK (jsonb_typeof(source_refs_json) = 'array');
