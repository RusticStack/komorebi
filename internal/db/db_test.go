package db

import "testing"

func TestOpenAppliesMigrationsAndSeedsLocalCache(t *testing.T) {
	dir := t.TempDir()
	database, err := Open(dir)
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}

	if err := EnsureSeedAccount(database); err != nil {
		t.Fatalf("EnsureSeedAccount() error = %v", err)
	}

	var accounts int
	if err := database.QueryRow(`SELECT COUNT(*) FROM accounts`).Scan(&accounts); err != nil {
		t.Fatalf("count accounts: %v", err)
	}
	if accounts != 1 {
		t.Fatalf("accounts = %d, want 1", accounts)
	}

	var folders int
	if err := database.QueryRow(`SELECT COUNT(*) FROM folders WHERE account_id = 'demo-local'`).Scan(&folders); err != nil {
		t.Fatalf("count folders: %v", err)
	}
	if folders != 5 {
		t.Fatalf("folders = %d, want 5", folders)
	}

	rows, err := database.Query(`PRAGMA table_info(accounts)`)
	if err != nil {
		t.Fatalf("table info: %v", err)
	}
	defer rows.Close()

	columns := map[string]bool{}
	for rows.Next() {
		var cid int
		var name, typ string
		var notNull, pk int
		var defaultValue any
		if err := rows.Scan(&cid, &name, &typ, &notNull, &defaultValue, &pk); err != nil {
			t.Fatalf("scan column: %v", err)
		}
		columns[name] = true
	}
	for _, name := range []string{"imap_host", "imap_port", "imap_security", "smtp_host", "smtp_port", "smtp_security"} {
		if !columns[name] {
			t.Fatalf("accounts missing column %q", name)
		}
	}
	if _, err := database.Exec(`
		INSERT INTO sync_state (account_id, folder_name, uid_validity, last_uid, highest_modseq)
		VALUES ('demo-local', 'INBOX', 1, 2, 3)
	`); err != nil {
		t.Fatalf("insert sync_state: %v", err)
	}

	if err := database.Close(); err != nil {
		t.Fatalf("close first database: %v", err)
	}

	reopened, err := Open(dir)
	if err != nil {
		t.Fatalf("Open() after migrations error = %v", err)
	}
	defer reopened.Close()

	var applied int
	if err := reopened.QueryRow(`SELECT COUNT(*) FROM schema_migrations`).Scan(&applied); err != nil {
		t.Fatalf("count migrations: %v", err)
	}
	if applied != 6 {
		t.Fatalf("applied migrations = %d, want 6", applied)
	}
}
