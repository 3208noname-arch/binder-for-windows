; Binder for Windows のインストーラー。ユーザー単位で入れるので、管理者の権限は要らない。
; GitHub Actions で次のように作る: ISCC.exe /DAppVersion=0.1.0 installer\binder.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{AA790BA5-B64B-442B-A159-CBE23A326F52}
AppName=Binder
AppVersion={#AppVersion}
AppVerName=Binder {#AppVersion}
AppPublisher=Ponzu
AppPublisherURL=https://github.com/3208noname-arch/binder-for-windows
AppSupportURL=https://github.com/3208noname-arch/binder-for-windows/issues
PrivilegesRequired=lowest
DefaultDirName={autopf}\Binder
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=Binder-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\binder.exe
UninstallDisplayName=Binder
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\binder.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ja.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Binder"; Filename: "{app}\binder.exe"
Name: "{autodesktop}\Binder"; Filename: "{app}\binder.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\binder.exe"; Description: "{cm:LaunchProgram,Binder}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; 既定のファイラーにしていれば、エクスプローラーへ戻す。設定の %LOCALAPPDATA%\binder は消さない
Filename: "{app}\binder.exe"; Parameters: "--restore-explorer"; Flags: runhidden waituntilterminated; RunOnceId: "RestoreExplorer"
