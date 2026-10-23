; Komorebi installer (Inno Setup 6.3 or later). Built by
; scripts/build-installer.ps1 and .github/workflows/release.yml:
;
;   ISCC [/DAppVersion=0.7.0] [/DBinDir=<folder>] installer\komorebi.iss
;
; The version defaults to the one inside komorebi.exe (BinDir, default
; target\release). Output: dist\komorebi-<version>-setup.exe.
;
; Per-user by default (no administrator rights, installs into
; %LOCALAPPDATA%\Programs\Komorebi); the first page offers "for all users",
; which elevates and installs into Program Files. Start menu shortcut, an
; optional desktop shortcut, an optional "Start Komorebi with Windows"
; (the same HKCU Run value, "Komorebi", and --minimized argument the app's
; own Settings switch writes), and an uninstaller entry in Settings > Apps.
; Komorebi is registered as an available mail app (Windows 11: Settings >
; Apps > Default apps) but is never made the default by the installer: the
; user picks it there. To pin it: right-click it in the Start menu, Pin to
; taskbar (Windows does not allow an installer to do that).
; /SILENT and /VERYSILENT install without questions (/ALLUSERS or /CURRENTUSER
; chooses the scope). A running Komorebi is closed first and restarted after
; an update. The uninstaller asks (default No) whether to delete the mail
; cache folder %LOCALAPPDATA%\komorebi too; a silent uninstall keeps it.
; Saved passwords stay in Windows Credential Manager (see PRIVACY.md).

#if Ver < EncodeVer(6, 3, 0, 0)
  #error Inno Setup 6.3 or later is needed (x64compatible)
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif
#ifndef AppVersion
  #define AppVersion GetVersionNumbersString(AddBackslash(SourcePath) + BinDir + "\komorebi.exe")
#endif

[Setup]
; Never change AppId: upgrades and the uninstaller find the install by it.
AppId={{6F0C2B7E-3C1D-4E8A-9B52-7A4D1E0F9C31}
AppName=Komorebi
AppVersion={#AppVersion}
AppVerName=Komorebi {#AppVersion}
AppPublisher=The Komorebi Authors
AppPublisherURL=https://github.com/RusticStack/komorebi
AppSupportURL=https://github.com/RusticStack/komorebi/issues
AppUpdatesURL=https://github.com/RusticStack/komorebi/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Komorebi
DefaultGroupName=Komorebi
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; A running Komorebi (it may sit in the tray) is closed through the Restart
; Manager before its files change (the app agrees to WM_QUERYENDSESSION and
; quits on WM_ENDSESSION, tray or not, src/win_shell.rs). Restart Manager
; only restarts programs registered for it, so a silent update that asks
; for /RESTARTAPPLICATIONS starts Komorebi itself ([Run], [Code]). Every
; running Komorebi also holds the fixed mutex Komorebi.App (src/main.rs,
; beside its per-cache single-instance mutex); the self-update closes it
; just before it starts this installer, so AppMutex only stops an install
; that would replace a Komorebi still running.
AppMutex=Komorebi.App,Global\Komorebi.App
CloseApplications=yes
CloseApplicationsFilter=komorebi.exe
RestartApplications=yes
SetupIconFile=..\assets\komorebi.ico
UninstallDisplayIcon={app}\komorebi.exe
UninstallDisplayName=Komorebi
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=komorebi-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
; The official European Portuguese translation (AO90) that ships with Inno Setup.
Name: "portuguese"; MessagesFile: "compiler:Languages\Portuguese.isl"

[Messages]
; The stock text does not say where a tray-only app is: it has no window to close.
english.SetupAppRunningError=Setup has detected that Komorebi is running.%n%nPlease close it now (if it has no window, right-click the Komorebi icon in the notification area and choose Quit), then click OK to continue, or Cancel to exit.
portuguese.SetupAppRunningError=O Setup detetou que o Komorebi está em execução.%n%nFeche-o agora (se não tiver janela, clique com o botão direito no ícone do Komorebi na área de notificação e escolha Sair) e clique em OK para continuar, ou em Cancelar para sair.
english.UninstallAppRunningError=Uninstall has detected that Komorebi is running.%n%nPlease close it now (if it has no window, right-click the Komorebi icon in the notification area and choose Quit), then click OK to continue, or Cancel to exit.
portuguese.UninstallAppRunningError=O Desinstalar detetou que o Komorebi está em execução.%n%nFeche-o agora (se não tiver janela, clique com o botão direito no ícone do Komorebi na área de notificação e escolha Sair) e clique em OK para continuar, ou em Cancelar para sair.

[CustomMessages]
english.TaskAutostart=Start Komorebi with Windows (in the notification area)
portuguese.TaskAutostart=Iniciar o Komorebi com o Windows (na área de notificação)
english.StartKomorebi=Start Komorebi
portuguese.StartKomorebi=Iniciar o Komorebi
english.AlsoDeleteMail=Also delete the mail Komorebi downloaded to this computer?%n%nThis deletes the folder %1 (downloaded mail, unsent drafts and Outbox messages, settings and logs). Mail on the server is not affected.%n%nSaved passwords stay in Windows Credential Manager: remove the entries named account:<number>:password.komorebi under Windows Credentials there. (Settings > Privacy > Remove all data in Komorebi deletes them, and this folder, before an uninstall.)
portuguese.AlsoDeleteMail=Eliminar também o correio que o Komorebi transferiu para este computador?%n%nIsto elimina a pasta %1 (correio transferido, rascunhos por enviar e mensagens da Caixa de saída, definições e registos). O correio no servidor não é afetado.%n%nAs palavras-passe guardadas ficam no Gestor de Credenciais do Windows: remova aí, em Credenciais do Windows, as entradas com o nome account:<número>:password.komorebi. (Definições > Privacidade > Remover todos os dados no Komorebi elimina-as, e esta pasta, antes de desinstalar.)

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "{cm:TaskAutostart}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinDir}\komorebi.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"; Flags: ignoreversion
Source: "..\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Komorebi"; Filename: "{app}\komorebi.exe"; Comment: "Komorebi"
Name: "{autodesktop}\Komorebi"; Filename: "{app}\komorebi.exe"; Tasks: desktopicon

[Registry]
; Same value and argument as src/win_shell.rs and src/notify.rs. Always the
; current user's Run key: autostart is a per-user choice.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Komorebi"; ValueData: """{app}\komorebi.exe"" --minimized"; Tasks: autostart; Flags: uninsdeletevalue

; Available mail client. HKA is HKCU for a per-user install and HKLM for an
; all-users one. The default is never set here.
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi"; ValueType: string; ValueName: ""; ValueData: "Komorebi"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Komorebi"
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Komorebi, a small and fast email client"
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi\Capabilities\URLAssociations"; ValueType: string; ValueName: "mailto"; ValueData: "Komorebi.Url.mailto"
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\komorebi.exe,0"
Root: HKA; Subkey: "Software\Clients\Mail\Komorebi\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\komorebi.exe"""
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Komorebi"; ValueData: "Software\Clients\Mail\Komorebi\Capabilities"; Flags: uninsdeletevalue

; The mailto: handler class the capability points at.
Root: HKA; Subkey: "Software\Classes\Komorebi.Url.mailto"; ValueType: string; ValueName: ""; ValueData: "URL:MailTo Protocol"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Komorebi.Url.mailto"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Komorebi.Url.mailto"; ValueType: string; ValueName: "FriendlyTypeName"; ValueData: "Komorebi"
Root: HKA; Subkey: "Software\Classes\Komorebi.Url.mailto\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\komorebi.exe,0"
Root: HKA; Subkey: "Software\Classes\Komorebi.Url.mailto\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\komorebi.exe"" ""%1"""

[Run]
Filename: "{app}\komorebi.exe"; Description: "{cm:StartKomorebi}"; Flags: nowait postinstall skipifsilent
; The self-update closes the app, runs this setup silently with
; /RESTARTAPPLICATIONS and expects Komorebi back: Restart Manager does not
; restart a program that is gone or never registered, so start it here, as
; the user who ran setup (never elevated, also for an all-users install).
Filename: "{app}\komorebi.exe"; Flags: nowait runasoriginaluser; Check: RestartAfterSilentUpdate

[Code]
// True when the command line holds the switch /Name (any case).
function SwitchGiven(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/' + Name) = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

// Silent setup asked to restart applications (what the self-update passes).
function RestartAfterSilentUpdate: Boolean;
begin
  Result := WizardSilent and SwitchGiven('RESTARTAPPLICATIONS');
end;

// After the app's files are removed: offer to delete the mail cache too.
// No is the default (Enter keeps it), and a silent uninstall keeps it.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Cache: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;
  Cache := ExpandConstant('{localappdata}\komorebi');
  if not DirExists(Cache) then
    Exit;
  if SuppressibleMsgBox(FmtMessage(CustomMessage('AlsoDeleteMail'), [Cache]),
       mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
    DelTree(Cache, True, True, True);
end;
