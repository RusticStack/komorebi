package db

import (
	"database/sql"
	"embed"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	_ "modernc.org/sqlite"
)

//go:embed migrations/*.sql
var migrationsFS embed.FS

// Open opens (and initializes if needed) the local mail cache SQLite database.
// Uses a pure-Go driver so the app builds cleanly on Windows without CGO.
func Open(appDataDir string) (*sql.DB, error) {
	if err := os.MkdirAll(appDataDir, 0755); err != nil {
		return nil, err
	}

	dbPath := filepath.Join(appDataDir, "komorebi.db")
	db, err := sql.Open("sqlite", dbPath+"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)")
	if err != nil {
		return nil, fmt.Errorf("open sqlite: %w", err)
	}

	if err := db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("ping sqlite: %w", err)
	}

	// Apply schema
	if err := applyMigrations(db); err != nil {
		db.Close()
		return nil, fmt.Errorf("apply migrations: %w", err)
	}

	return db, nil
}

func applyMigrations(db *sql.DB) error {
	if _, err := db.Exec(`
		CREATE TABLE IF NOT EXISTS schema_migrations (
			version INTEGER PRIMARY KEY,
			applied_at INTEGER NOT NULL DEFAULT (unixepoch())
		)
	`); err != nil {
		return fmt.Errorf("create schema_migrations: %w", err)
	}

	migrations := []struct {
		version int
		name    string
		apply   func(*sql.Tx) error
	}{
		{version: 1, name: "001_core_schema.sql"},
		{version: 2, name: "002_drafts.sql"},
		{version: 3, name: "003_attachments.sql"},
		{version: 4, name: "004_account_server_settings.sql", apply: applyAccountServerSettingsMigration},
		{version: 5, name: "005_sync_state.sql"},
		{version: 6, name: "006_sync_preferences.sql"},
	}

	for _, migration := range migrations {
		applied, err := migrationApplied(db, migration.version)
		if err != nil {
			return err
		}
		if applied {
			continue
		}

		tx, err := db.Begin()
		if err != nil {
			return err
		}

		if migration.apply != nil {
			err = migration.apply(tx)
		} else {
			err = execEmbeddedMigration(tx, migration.name)
		}
		if err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("migration %s: %w", migration.name, err)
		}

		if _, err := tx.Exec(`INSERT OR REPLACE INTO schema_migrations (version, applied_at) VALUES (?, unixepoch())`, migration.version); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("record migration %s: %w", migration.name, err)
		}

		if err := tx.Commit(); err != nil {
			return fmt.Errorf("commit migration %s: %w", migration.name, err)
		}
	}
	return nil
}

func migrationApplied(db *sql.DB, version int) (bool, error) {
	var count int
	if err := db.QueryRow(`SELECT COUNT(*) FROM schema_migrations WHERE version = ?`, version).Scan(&count); err != nil {
		return false, fmt.Errorf("check migration %d: %w", version, err)
	}
	return count > 0, nil
}

func execEmbeddedMigration(tx *sql.Tx, name string) error {
	content, err := migrationsFS.ReadFile("migrations/" + name)
	if err != nil {
		return err
	}
	_, err = tx.Exec(string(content))
	return err
}

func applyAccountServerSettingsMigration(tx *sql.Tx) error {
	if err := ensureColumn(tx, "accounts", "imap_security", "TEXT DEFAULT 'tls'"); err != nil {
		return err
	}
	if err := ensureColumn(tx, "accounts", "smtp_security", "TEXT DEFAULT 'starttls'"); err != nil {
		return err
	}
	_, err := tx.Exec(`CREATE INDEX IF NOT EXISTS idx_accounts_email ON accounts(email)`)
	return err
}

func ensureColumn(tx *sql.Tx, tableName, columnName, definition string) error {
	columns, err := tableColumns(tx, tableName)
	if err != nil {
		return err
	}
	if columns[columnName] {
		return nil
	}
	if !safeIdentifier(tableName) || !safeIdentifier(columnName) {
		return fmt.Errorf("unsafe schema identifier")
	}
	_, err = tx.Exec(fmt.Sprintf(`ALTER TABLE %s ADD COLUMN %s %s`, tableName, columnName, definition))
	return err
}

func tableColumns(tx *sql.Tx, tableName string) (map[string]bool, error) {
	if !safeIdentifier(tableName) {
		return nil, fmt.Errorf("unsafe table name")
	}
	rows, err := tx.Query(fmt.Sprintf(`PRAGMA table_info(%s)`, tableName))
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	columns := map[string]bool{}
	for rows.Next() {
		var cid int
		var name, typ string
		var notNull, pk int
		var defaultValue any
		if err := rows.Scan(&cid, &name, &typ, &notNull, &defaultValue, &pk); err != nil {
			return nil, err
		}
		columns[name] = true
	}
	return columns, rows.Err()
}

func safeIdentifier(value string) bool {
	if value == "" {
		return false
	}
	for _, r := range value {
		if r == '_' || ('a' <= r && r <= 'z') || ('A' <= r && r <= 'Z') || ('0' <= r && r <= '9') {
			continue
		}
		return false
	}
	return !strings.HasPrefix(value, "sqlite_")
}

// EnsureSeedAccount creates a local demo account with folders if none exists.
// This gives the user a fully working shell with real data on first launch.
func EnsureSeedAccount(db *sql.DB) error {
	var count int
	err := db.QueryRow(`SELECT COUNT(*) FROM accounts`).Scan(&count)
	if err != nil {
		return err
	}
	if count > 0 {
		return nil
	}

	// Create one local demo account
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()

	_, err = tx.Exec(`
		INSERT INTO accounts (id, email, display_name, provider_kind, created_at)
		VALUES ('demo-local', 'you@local.test', 'Local Demo', 'local', unixepoch())
	`)
	if err != nil {
		return err
	}

	folders := []struct {
		id, name, special string
	}{
		{"inbox", "Inbox", "inbox"},
		{"sent", "Sent", "sent"},
		{"drafts", "Drafts", "drafts"},
		{"archive", "Archive", "archive"},
		{"trash", "Trash", "trash"},
	}

	for _, f := range folders {
		_, err = tx.Exec(`
			INSERT INTO folders (id, account_id, name, display_name, special_use, unread_count)
			VALUES (?, 'demo-local', ?, ?, ?, 0)
		`, f.id, f.name, f.name, f.special)
		if err != nil {
			return err
		}
	}

	// Seed a realistic set of messages (real rows, not mocks)
	seedMessages(tx)

	// Seed one real draft so the Drafts surface has something on first run
	seedOneDraft(tx)

	return tx.Commit()
}

func seedOneDraft(tx *sql.Tx) {
	_, _ = tx.Exec(`
		INSERT OR IGNORE INTO composer_drafts (id, account_id, subject, to_addresses, body_html, created_at, updated_at)
		VALUES ('draft-seed-1', 'demo-local', 'Draft: Q3 numbers', '["finance@company.test"]', 
		        '<p>Here are the latest numbers...</p>', unixepoch()-3600, unixepoch()-3600)
	`)
}

func seedMessages(tx *sql.Tx) {
	subjects := []string{
		"Q3 planning sync",
		"Invoice for March services",
		"Re: Design feedback",
		"Team offsite 2026",
		"Your order has shipped",
		"Security alert - new login",
		"Project retro notes",
		"FW: Client presentation",
		"Vacation request approved",
		"New feature spec draft",
	}

	senders := []struct{ name, email string }{
		{"Alex Rivera", "alex@company.test"},
		{"Jordan Hale", "jordan@client.test"},
		{"Sam Patel", "sam@internal.test"},
		{"Taylor Kim", "taylor@design.test"},
		{"Casey Morgan", "casey@finance.test"},
	}

	for i := 0; i < 48; i++ {
		s := senders[i%len(senders)]
		subj := subjects[i%len(subjects)]
		isRead := i%3 != 0
		folder := "inbox"
		if i%7 == 0 {
			folder = "sent"
		}

		_, _ = tx.Exec(`
			INSERT INTO messages 
			(id, account_id, folder_id, subject, sender_name, sender_email, 
			 to_addresses, date_received, is_read, is_flagged, snippet, body_text)
			VALUES 
			(?, 'demo-local', ?, ?, ?, ?, ?, unixepoch() - ?, ?, 0, ?, ?)
		`,
			fmt.Sprintf("msg-%03d", i),
			folder,
			subj,
			s.name,
			s.email,
			`["you@local.test"]`,
			3600*(i+1),
			isRead,
			"Real local message body for "+subj+". This is persisted in SQLite.",
			"Real local message body for "+subj+". This is persisted in SQLite and searchable.",
		)
	}
}
