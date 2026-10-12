; Inno Setup script for the Windows installer.
;
;   iscc /DAppVersion=1.0.0 /DExePath=..\..\target\release\avrcontrol.exe packaging\windows\avrcontrol.iss
;
; Installs per user (no admin prompt) to %LOCALAPPDATA%\Programs\AVR Control.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef ExePath
  #define ExePath "..\..\target\release\avrcontrol.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\target\installer"
#endif

[Setup]
AppId={{CC8199A3-2260-4EF2-9AF4-56BEAC6272E5}
AppName=AVR Control
AppVersion={#AppVersion}
AppVerName=AVR Control {#AppVersion}
AppPublisher=Virmin8
AppPublisherURL=https://github.com/Virmin8/AVRControl
AppSupportURL=https://github.com/Virmin8/AVRControl/issues
DefaultDirName={autopf}\AVR Control
DefaultGroupName=AVR Control
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\avrcontrol.exe
UninstallDisplayName=AVR Control
OutputDir={#OutputDir}
OutputBaseFilename=avrcontrol-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Tasks]
Name: "startup"; Description: "Start AVR Control when I sign in"

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "..\..\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"

[Icons]
Name: "{autoprograms}\AVR Control"; Filename: "{app}\avrcontrol.exe"

[Registry]
; Same value the app's own "Start on login" switch writes (src/autostart.rs),
; so the switch in Settings reflects this choice.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AVRControl"; ValueData: """{app}\avrcontrol.exe"" daemon"; Tasks: startup; Flags: uninsdeletevalue

[Run]
Filename: "{app}\avrcontrol.exe"; Description: "Start AVR Control"; Flags: nowait postinstall skipifsilent
; In-app updates run this installer with /SILENT after quitting the app; start it again.
Filename: "{app}\avrcontrol.exe"; Parameters: "daemon"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "{app}\avrcontrol.exe"; Parameters: "quit"; Flags: runhidden; RunOnceId: "QuitApp"

[Code]
// Stop a running copy before replacing it on upgrade.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\avrcontrol.exe');
  if FileExists(Exe) then
  begin
    Exec(Exe, 'quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
    Sleep(1000);
  end;
  Result := '';
end;

// The app's own "Start on login" switch can add the Run entry without the
// installer knowing, so remove it on uninstall either way.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'AVRControl');
end;
