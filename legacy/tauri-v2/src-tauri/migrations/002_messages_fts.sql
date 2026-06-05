CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject,
  sender_name,
  sender_email,
  body_text,
  body_html,
  content='messages',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_after_insert
AFTER INSERT ON messages
FOR EACH ROW
BEGIN
  INSERT INTO messages_fts (
    rowid,
    subject,
    sender_name,
    sender_email,
    body_text,
    body_html
  )
  VALUES (
    NEW.rowid,
    NEW.subject,
    NEW.sender_name,
    NEW.sender_email,
    NEW.body_text,
    NEW.body_html
  );
END;

CREATE TRIGGER messages_fts_after_delete
AFTER DELETE ON messages
FOR EACH ROW
BEGIN
  INSERT INTO messages_fts (
    messages_fts,
    rowid,
    subject,
    sender_name,
    sender_email,
    body_text,
    body_html
  )
  VALUES (
    'delete',
    OLD.rowid,
    OLD.subject,
    OLD.sender_name,
    OLD.sender_email,
    OLD.body_text,
    OLD.body_html
  );
END;

CREATE TRIGGER messages_fts_after_update
AFTER UPDATE OF subject, sender_name, sender_email, body_text, body_html ON messages
FOR EACH ROW
BEGIN
  INSERT INTO messages_fts (
    messages_fts,
    rowid,
    subject,
    sender_name,
    sender_email,
    body_text,
    body_html
  )
  VALUES (
    'delete',
    OLD.rowid,
    OLD.subject,
    OLD.sender_name,
    OLD.sender_email,
    OLD.body_text,
    OLD.body_html
  );

  INSERT INTO messages_fts (
    rowid,
    subject,
    sender_name,
    sender_email,
    body_text,
    body_html
  )
  VALUES (
    NEW.rowid,
    NEW.subject,
    NEW.sender_name,
    NEW.sender_email,
    NEW.body_text,
    NEW.body_html
  );
END;

INSERT INTO messages_fts (messages_fts) VALUES ('rebuild');
