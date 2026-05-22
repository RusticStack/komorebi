ALTER TABLE folders
ADD COLUMN attributes_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(attributes_json));
