; Komorebi's per-user installer (Inno Setup 6). Built by
; .github/workflows/release.yml:
;
;   ISCC /DAppVersion=0.3.0 installer\komorebi.iss
;   (/DBinDir=<folder> takes komorebi.exe from elsewhere; default target\release)
;
; No administrator rights: the app goes to %LOCALAPPDATA%\Programs\Komorebi
; with a Start menu shortcut and an uninstaller (registered under HKCU).
; /SILENT and /VERYSILENT install without questions. The uninstaller asks
; (default No) whether to delete the mail cache folder
; %LOCALAPPDATA%\komorebi too; a silent uninstall keeps it. Saved passwords
; stay in Windows Credential Manager: Settings > Privacy > Remove all data
; deletes them (see PRIVACY.md).

#if Ver < EncodeVer(6, 3, 0, 0)
  #error Inno Setup 6.3 or later is needed (x64compatible)
#endif
#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=x.y.z
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
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
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Komorebi
DefaultGroupName=Komorebi
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; A running Komorebi is closed (Restart Manager) before its files change.
CloseApplications=yes
RestartApplications=no
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

[Files]
Source: "{#BinDir}\komorebi.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"; Flags: ignoreversion
Source: "..\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Komorebi"; Filename: "{app}\komorebi.exe"

[Run]
Filename: "{app}\komorebi.exe"; Description: "Start Komorebi"; Flags: nowait postinstall skipifsilent

[Code]
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
  if SuppressibleMsgBox(
       'Also delete the mail Komorebi downloaded to this computer?' + #13#10 + #13#10 +
       'This deletes the folder ' + Cache + ' (downloaded mail, unsent drafts and ' +
       'Outbox messages, settings and logs). Mail on the server is not affected.' + #13#10 + #13#10 +
       'Saved passwords stay in Windows Credential Manager: remove the entries ' +
       'named account:<number>:password.komorebi under Windows Credentials there. ' +
       '(Settings > Privacy > Remove all data in Komorebi deletes them, and this ' +
       'folder, before an uninstall.)',
       mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
    DelTree(Cache, True, True, True);
end;
