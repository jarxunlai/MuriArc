ALTER TABLE ai_conversation_messages
    ADD COLUMN source_refs_json TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(source_refs_json) AND json_type(source_refs_json) = 'array');
