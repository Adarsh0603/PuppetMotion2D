; Windows installer for PuppetMotion 2D (Inno Setup 6).
;
; Installs for the current user only (no admin prompt) into
; %LOCALAPPDATA%\Programs\PuppetMotion 2D, adds a Start menu shortcut, and
; opens .pm2d projects with the app, shown with their own icon. The uninstaller
; removes all of that again (saved motions, templates and recovery copies in
; %LOCALAPPDATA%\PuppetMotion2D are left alone).
;
; Build the app first, then from the repository root:
;   cargo build --release -p puppetmotion
;   iscc /DAppVersion=0.2.0 installer\puppetmotion.iss
; The setup lands in target\installer\.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "PuppetMotion 2D"
#define AppExe "PuppetMotion2D.exe"
#define ProgId "PuppetMotion2D.Project"

[Setup]
; Keep this id: it's how Windows knows a new version updates the installed one.
AppId={{24C03B0B-AC5F-4041-8340-4496D08B4BEC}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=PuppetMotion 2D
AppPublisherURL=https://github.com/Adarsh0603/PuppetMotion2D
AppSupportURL=https://github.com/Adarsh0603/PuppetMotion2D/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ChangesAssociations=yes
SetupIconFile=..\app\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=PuppetMotion2D-Setup-{#AppVersion}

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\app\assets\pm2d.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; .pm2d files open with the app and show the project icon. HKA is the
; current user's classes here, since the install is per user.
Root: HKA; Subkey: "Software\Classes\.pm2d"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.pm2d\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "{#AppName} project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\pm2d.ico"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".pm2d"; ValueData: ""; Flags: uninsdeletekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
