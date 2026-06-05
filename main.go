package main

import (
	"embed"
	"log"
	"os"
	"path/filepath"

	"komorebi/internal/db"
	"komorebi/internal/mail"

	"github.com/wailsapp/wails/v3/pkg/application"
)

//go:embed all:frontend/dist
var assets embed.FS

func main() {
	// Real local app data directory (Windows-friendly)
	appData, err := os.UserConfigDir()
	if err != nil {
		appData = "."
	}
	cacheDir := filepath.Join(appData, "Komorebi", "cache")

	database, err := db.Open(cacheDir)
	if err != nil {
		log.Fatalf("failed to open local mail cache: %v", err)
	}
	defer database.Close()

	mailService := mail.NewService(database)

	app := application.New(application.Options{
		Name:        "Komorebi",
		Description: "Privacy-first local-first desktop email client",
		Services: []application.Service{
			application.NewService(mailService),
		},
		Assets: application.AssetOptions{
			Handler: application.AssetFileServerFS(assets),
		},
		Mac: application.MacOptions{
			ApplicationShouldTerminateAfterLastWindowClosed: true,
		},
	})

	// Create the main window with Windows-native custom chrome expectations
	app.Window.NewWithOptions(application.WebviewWindowOptions{
		Name:   "main",
		Title:  "Komorebi",
		Width:  1280,
		Height: 800,
		// We will drive a custom titlebar from Solid (matching original Japandi direction)
		Frameless: true,
	})

	if err := app.Run(); err != nil {
		log.Fatal(err)
	}
}
