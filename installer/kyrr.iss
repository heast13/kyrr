; Inno Setup script for kyrr. Built by scripts/package.py, which passes
; /DAppVersion and /DStage (the staged files).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Stage
  #define Stage "..\dist\stage"
#endif

[Setup]
AppId={{6B7C2E0A-4F1D-4C8B-9A5E-2D3F1B6C7A90}
AppName=kyrr
AppVersion={#AppVersion}
AppPublisher=heast13
AppPublisherURL=https://github.com/heast13/kyrr
DefaultDirName={autopf}\kyrr
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
WizardStyle=modern
SetupIconFile=..\crates\kyrr-app\ui\icons\kyrr.ico
UninstallDisplayIcon={app}\kyrr.exe
OutputDir=..\dist
OutputBaseFilename=kyrr-setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
en.AudioNote=kyrr adds a noise filter to the Windows audio system. Like Equalizer APO, it switches on the Windows setting that allows audio filters not signed by Microsoft. Uninstalling kyrr restores the setting.
de.AudioNote=kyrr fügt dem Windows-Audiosystem einen Rauschfilter hinzu. Wie Equalizer APO schaltet es dafür die Windows-Einstellung ein, die nicht von Microsoft signierte Audiofilter erlaubt. Beim Deinstallieren stellt kyrr die Einstellung wieder her.
en.StartKyrr=Start kyrr
de.StartKyrr=kyrr starten

[Files]
Source: "{#Stage}\kyrr.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\kyrr_apo.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet8_48khz_hr.onnx"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet8_48khz_hr.weights.f32"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet2_48khz_hr.onnx"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet2_48khz_hr.weights.f32"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\kyrr"; Filename: "{app}\kyrr.exe"

[Registry]
; kyrr registers itself for sign-in after the first-run setup; remove that on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "kyrr"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\kyrr.exe"; Parameters: "--setup install ""{app}\kyrr_apo.dll"""; StatusMsg: "Setting up the audio filter..."; Flags: runhidden waituntilterminated
Filename: "{sys}\net.exe"; Parameters: "start audiosrv"; Flags: runhidden waituntilterminated
Filename: "{app}\kyrr.exe"; Description: "{cm:StartKyrr}"; Flags: postinstall nowait runasoriginaluser skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im kyrr.exe"; Flags: runhidden waituntilterminated; RunOnceId: "StopKyrr"
Filename: "{app}\kyrr.exe"; Parameters: "--setup uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveFilter"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\kyrr"
Type: filesandordirs; Name: "{userappdata}\kyrr"

[Code]
function InitializeSetup(): Boolean;
begin
  MsgBox(CustomMessage('AudioNote'), mbInformation, MB_OK);
  Result := True;
end;

// The audio engine keeps kyrr_apo.dll loaded; stop it (and a running kyrr)
// so an update can replace the files. [Run] starts the audio service again.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kyrr.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\net.exe'), 'stop audiosrv /y', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
