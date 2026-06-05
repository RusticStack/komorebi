ALTER TABLE messages
  ADD COLUMN mime_parse_warnings_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(mime_parse_warnings_json));
