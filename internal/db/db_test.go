package db

import "testing"

func TestOpenAppliesMigrationsAndSeedsLocalCache(t *testing.T) {
	database, err := Open(t.TempDir())
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	defer database.Close()

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
}
