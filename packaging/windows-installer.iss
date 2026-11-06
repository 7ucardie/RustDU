; RustDU setup for Windows: per user, no admin rights. Built by windows-installer.ps1, which runs:
;   ISCC /DAppVersion=<version> /DExe=<path to rustdu.exe> /O<output dir> packaging\windows-installer.iss

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef Exe
  #error Pass /DExe=<path to rustdu.exe>
#endif

[Setup]
; Never change AppId: Windows uses it to recognise an installed RustDU for upgrades and uninstall.
AppId={{7D2B1C4E-9A63-4F0B-B5E2-3C8D61F0A917}
AppName=RustDU
AppVersion={#AppVersion}
AppPublisher=Ellert van der Vecht
AppPublisherURL=https://github.com/7ucardie/RustDU
AppSupportURL=https://github.com/7ucardie/RustDU/issues
DefaultDirName={localappdata}\Programs\rustdu
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=rustdu-{#AppVersion}-windows-amd64-setup
SetupIconFile=..\assets\disk-explorer.ico
UninstallDisplayIcon={app}\rustdu.exe
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\RustDU"; Filename: "{app}\rustdu.exe"
Name: "{autodesktop}\RustDU"; Filename: "{app}\rustdu.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\rustdu.exe"; Description: "{cm:LaunchProgram,RustDU}"; Flags: nowait postinstall skipifsilent
