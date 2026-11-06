; asides' installer for Windows (Inno Setup 6), compiled by scripts\package-windows.ps1 from the
; folder it stages:
;
;   ISCC /DVersion=X.Y.Z /DArch=arm64|x64 /DSource=STAGED_DIR /DOutput=OUT_DIR /DName=NAME asides.iss
;     -> OUT_DIR\NAME-setup.exe
;
; Per user, with no administrator (PrivilegesRequired=lowest), as asides' data is: asides-app.exe,
; asides-overlay.exe and asides.exe in %LOCALAPPDATA%\Programs\asides; asides in the Start menu
; and in Settings > Apps, under the key io.github.mihaicristianfarcas.asides_is1 in HKCU's
; Uninstall. The app puts its folder on the user's PATH the first time it runs (the `asides`
; command) and registers asides:// links each time.
;
; Uninstalling quits asides first, then takes away what it made beside its files: its folder on
; the PATH, asides:// links and its start at login. It keeps the settings, the voice models and
; the log (%LOCALAPPDATA%\asides) and the history (%USERPROFILE%\.asides), as the Mac app's
; uninstall does.
;
; /DSign signs Setup and the uninstaller with the sign tool named ssign (package-windows.ps1 -Sign
; defines it: Certum's cloud certificate, through ssign). The programs are signed before they get
; here. An update is this installer run again, silently, by the app (`update.rs`).

#ifndef Version
  #error Pass /DVersion=X.Y.Z, the workspace's version
#endif
#ifndef Arch
  #error Pass /DArch=arm64 or /DArch=x64, what the programs are built for
#endif
#ifndef Source
  #error Pass /DSource=DIR, the folder scripts\package-windows.ps1 stages
#endif
#ifndef Output
  #define Output "."
#endif
#ifndef Name
  #define Name "asides-" + Version + "-windows-" + Arch
#endif
; A debug build's programs are hundreds of megabytes: CI packages them with /DCompression=none
; rather than wait for LZMA.
#ifndef Compression
  #define Compression "lzma2/ultra64"
#endif
#if Arch == "arm64"
  #define Architectures "arm64"
#else
  #define Architectures "x64compatible"
#endif

[Setup]
; The name of asides' entry in Settings > Apps (Inno adds _is1), as the Mac app's bundle
; identifier: it must never change, or a new release installs beside the old one.
AppId=io.github.mihaicristianfarcas.asides
AppName=asides
AppVersion={#Version}
AppPublisher=asides
AppPublisherURL=https://asides.mihaicristian.dev
AppSupportURL=https://github.com/mihaicristianfarcas/asides/issues
AppUpdatesURL=https://asides.mihaicristian.dev
VersionInfoVersion={#Version}
VersionInfoProductName=asides
VersionInfoDescription=asides Setup
UninstallDisplayName=asides
UninstallDisplayIcon={app}\asides-app.exe
PrivilegesRequired=lowest
; {userpf} is %LOCALAPPDATA%\Programs, per user.
DefaultDirName={userpf}\asides
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed={#Architectures}
ArchitecturesInstallIn64BitMode={#Architectures}
; Windows 10 2004: the pill left out of captures (WDA_EXCLUDEFROMCAPTURE).
MinVersion=10.0.19041
; An asides that runs holds its files: the installer closes it (Restart Manager), and asks first
; unless it runs silently.
CloseApplications=force
RestartApplications=no
WizardStyle=modern
SetupIconFile=..\asides.ico
OutputDir={#Output}
OutputBaseFilename={#Name}-setup
Compression={#Compression}
SolidCompression=yes
#ifdef Sign
SignTool=ssign
SignedUninstaller=yes
#endif

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#Source}\asides-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\asides-overlay.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\asides.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE-APACHE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE-MIT.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\asides"; Filename: "{app}\asides-app.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\asides-app.exe"; WorkingDir: "{app}"; Description: "Open asides"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; asides quits as its menu's Quit does; its overlay ends with it. Waited for (the processes, not
; a timer), so their files can go.
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -NonInteractive -Command ""& '{app}\asides.exe' quit; Wait-Process -Name asides-app,asides-overlay -Timeout 10 -ErrorAction SilentlyContinue"""; \
  Flags: runhidden waituntilterminated; RunOnceId: "QuitAsides"

[UninstallDelete]
Type: dirifempty; Name: "{app}"

[Code]
const
  UserEnvironment = 'Environment';
  { Where the app makes itself start at login (crates/asides-app/src/windows/system.rs), and where
    Settings > Apps > Startup turns that off. }
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  StartupApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';
  { asides:// links (system.rs's register_links). }
  LinksKey = 'Software\Classes\asides';
  { HWND_BROADCAST is Inno's own. }
  WM_SETTINGCHANGE = $001A;
  SMTO_ABORTIFHUNG = $0002;

function SendMessageTimeout(Window: HWND; Msg: Cardinal; WParam: Cardinal; LParam: String;
  Flags: Cardinal; Timeout: Cardinal; var Answer: Cardinal): Cardinal;
  external 'SendMessageTimeoutW@user32.dll stdcall';

{ Whether Text names a file in this installation's folder (a command, quoted or not). }
function InApp(Text: String): Boolean;
begin
  Result := Pos(Lowercase(ExpandConstant('{app}\')), Lowercase(Text)) > 0;
end;

{ The user's PATH without this installation's folder; True when it was there. }
function WithoutApp(Path: String; var Rest: String): Boolean;
var
  App, Entry: String;
  At: Integer;
begin
  App := Lowercase(RemoveBackslashUnlessRoot(ExpandConstant('{app}')));
  Result := False;
  Rest := '';
  Path := Path + ';';
  while Length(Path) > 0 do
  begin
    At := Pos(';', Path);
    Entry := Copy(Path, 1, At - 1);
    Delete(Path, 1, At);
    if Lowercase(RemoveBackslashUnlessRoot(Trim(Entry))) = App then
      Result := True
    else if Entry <> '' then
    begin
      if Rest <> '' then Rest := Rest + ';';
      Rest := Rest + Entry;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Rest, Command: String;
  Answer: Cardinal;
begin
  if CurUninstallStep <> usPostUninstall then Exit;
  if RegQueryStringValue(HKCU, UserEnvironment, 'Path', Path) and WithoutApp(Path, Rest) then
  begin
    RegWriteExpandStringValue(HKCU, UserEnvironment, 'Path', Rest);
    SendMessageTimeout(HWND_BROADCAST, WM_SETTINGCHANGE, 0, 'Environment', SMTO_ABORTIFHUNG, 5000, Answer);
  end;
  if RegQueryStringValue(HKCU, RunKey, 'asides', Command) and InApp(Command) then
  begin
    RegDeleteValue(HKCU, RunKey, 'asides');
    RegDeleteValue(HKCU, StartupApprovedKey, 'asides');
  end;
  if RegQueryStringValue(HKCU, LinksKey + '\shell\open\command', '', Command) and InApp(Command) then
    RegDeleteKeyIncludingSubkeys(HKCU, LinksKey);
end;
