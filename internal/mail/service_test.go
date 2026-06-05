package mail

import (
	"testing"

	"komorebi/internal/db"
)

func newTestService(t *testing.T) *Service {
	t.Helper()

	database, err := db.Open(t.TempDir())
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	t.Cleanup(func() { database.Close() })

	return NewService(database)
}

func TestServiceLoadsSeededShellAndMutatesLocalCache(t *testing.T) {
	service := newTestService(t)

	shell, err := service.LoadMailShell()
	if err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}
	if len(shell.Accounts) != 1 {
		t.Fatalf("accounts = %d, want 1", len(shell.Accounts))
	}
	if len(shell.Folders) != 5 {
		t.Fatalf("folders = %d, want 5", len(shell.Folders))
	}
	if len(shell.Messages) == 0 {
		t.Fatal("messages = 0, want seeded messages")
	}

	messageID := shell.Messages[0].ID
	if err := service.MarkRead(messageID, false); err != nil {
		t.Fatalf("MarkRead(false) error = %v", err)
	}
	page, _, _, err := service.LoadMessagePage("inbox", 0, 1)
	if err != nil {
		t.Fatalf("LoadMessagePage() error = %v", err)
	}
	if len(page) != 1 {
		t.Fatalf("page length = %d, want 1", len(page))
	}

	results, err := service.SearchMail("planning", 10)
	if err != nil {
		t.Fatalf("SearchMail() error = %v", err)
	}
	if len(results) == 0 {
		t.Fatal("SearchMail() returned no results for seeded term")
	}
}

func TestServicePersistsDraftsAndCreatesLocalSentMessage(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}

	draft := Draft{
		ID:        "draft-test",
		AccountID: "demo-local",
		Subject:   "Draft smoke",
		To:        `["recipient@example.test"]`,
		BodyHTML:  "<p>Hello</p>",
	}
	if err := service.SaveDraft(draft); err != nil {
		t.Fatalf("SaveDraft() error = %v", err)
	}

	drafts, err := service.ListDrafts()
	if err != nil {
		t.Fatalf("ListDrafts() error = %v", err)
	}
	found := false
	for _, item := range drafts {
		if item.ID == draft.ID {
			found = true
		}
	}
	if !found {
		t.Fatal("saved draft not returned by ListDrafts()")
	}

	if err := service.SendDraft(draft.ID); err != nil {
		t.Fatalf("SendDraft() error = %v", err)
	}

	var sent int
	if err := service.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = 'sent-draft-test' AND folder_id = 'sent'`).Scan(&sent); err != nil {
		t.Fatalf("count sent message: %v", err)
	}
	if sent != 1 {
		t.Fatalf("sent rows = %d, want 1", sent)
	}
}
