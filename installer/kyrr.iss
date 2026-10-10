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
; Version info in the setup file itself; the code signing service checks it.
VersionInfoVersion={#AppVersion}
VersionInfoProductName=kyrr
VersionInfoProductTextVersion={#AppVersion}
VersionInfoCopyright=Copyright (c) 2026 heast13
AppPublisher=heast13
AppPublisherURL=https://github.com/heast13/kyrr
DefaultDirName={autopf}\kyrr
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 22H2 or newer; older builds are out of support.
MinVersion=10.0.19045
WizardStyle=modern
; The welcome page carries the note about the audio setting.
DisableWelcomePage=no
; The Windows display language picks German or English, no extra question.
ShowLanguageDialog=auto
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
en.AudioNote=kyrr hooks into Windows sound. For that it changes one Windows setting and briefly restarts the audio service, so sound drops out for a second or two. Uninstalling puts everything back.
de.AudioNote=kyrr klinkt sich in den Windows-Ton ein. Dafür stellt es eine Windows-Einstellung um und startet kurz den Audiodienst neu, der Ton setzt dabei ein, zwei Sekunden aus. Beim Deinstallieren wird alles zurückgestellt.
en.UpdateNote=kyrr %1 is already installed and will be updated. Settings and speech models stay.
de.UpdateNote=kyrr %1 ist schon installiert und wird aktualisiert. Einstellungen und Sprachmodelle bleiben erhalten.
en.StartKyrr=Start kyrr
de.StartKyrr=kyrr starten
en.SetupFailed=kyrr's audio filter could not be set up:%n%1%n%nThe app still works for dictation. Reinstalling usually helps.
de.SetupFailed=Der Audiofilter von kyrr ließ sich nicht einrichten:%n%1%n%nDiktieren geht trotzdem. Neu installieren hilft meistens.
en.RemoveData=Also delete the downloaded speech models (up to 900 MB) and kyrr's settings?%n%nSound drops out for a second while kyrr's filter is removed.
de.RemoveData=Auch die heruntergeladenen Sprachmodelle (bis zu 900 MB) und die Einstellungen von kyrr löschen?%n%nDer Ton setzt kurz aus, während der Filter entfernt wird.
en.UninstallFailed=Not everything could be removed:%n%1
de.UninstallFailed=Nicht alles ließ sich entfernen:%n%1

[Files]
Source: "{#Stage}\kyrr.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\kyrr_apo.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion
; The Visual C++ runtime onnxruntime.dll needs, app-local.
Source: "{#Stage}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\vcruntime140_1.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\msvcp140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\msvcp140_1.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet8_48khz_hr.onnx"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet8_48khz_hr.weights.f32"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet2_48khz_hr.onnx"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dpdfnet2_48khz_hr.weights.f32"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\kyrr"; Filename: "{app}\kyrr.exe"

[Registry]
; kyrr registers itself for sign-in after the first-run setup; remove that on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "kyrr"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\kyrr.exe"; Description: "{cm:StartKyrr}"; Flags: postinstall nowait runasoriginaluser skipifsilent

[Code]
const
  // Where Setup records an installed kyrr (AppId + "_is1").
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{6B7C2E0A-4F1D-4C8B-9A5E-2D3F1B6C7A90}_is1';

var
  AudioStopped: Boolean;
  RemoveData: Boolean;

// The notes go on the welcome page instead of a box before it: that an
// installed kyrr is updated in place, and what happens to the sound.
procedure InitializeWizard();
var
  Note, Version: String;
begin
  Note := CustomMessage('AudioNote');
  if RegQueryStringValue(HKLM, UninstallKey, 'DisplayVersion', Version) then
    Note := FmtMessage(CustomMessage('UpdateNote'), [Version]) + #13#10#13#10 + Note;
  WizardForm.WelcomeLabel2.Caption := WizardForm.WelcomeLabel2.Caption + #13#10#13#10 + Note;
end;

// On an update the audio engine keeps kyrr_apo.dll loaded: stop it (and a
// running kyrr) so the files can be replaced. A first install leaves the
// sound alone.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kyrr.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  if FileExists(ExpandConstant('{app}\kyrr_apo.dll')) then
  begin
    Exec(ExpandConstant('{sys}\net.exe'), 'stop audiosrv /y', '', SW_HIDE, ewWaitUntilTerminated, Code);
    AudioStopped := True;
  end;
  Result := '';
end;

procedure StartAudio();
var
  Code: Integer;
begin
  if AudioStopped then
  begin
    Exec(ExpandConstant('{sys}\net.exe'), 'start audiosrv', '', SW_HIDE, ewWaitUntilTerminated, Code);
    AudioStopped := False;
  end;
end;

// Registers the filter (kyrr finds its own DLL; it takes no path) and says
// so when that fails, instead of a quiet "finished".
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    if not Exec(ExpandConstant('{app}\kyrr.exe'), '--setup install', '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
      MsgBox(FmtMessage(CustomMessage('SetupFailed'), [IntToStr(Code)]), mbError, MB_OK);
    StartAudio();
  end;
end;

// Whatever happened (cancelled, failed), the sound comes back.
procedure DeinitializeSetup();
begin
  StartAudio();
end;

function InitializeUninstall(): Boolean;
begin
  // A silent uninstall keeps the models (no one to ask).
  RemoveData := not UninstallSilent and (MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO) = IDYES);
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im kyrr.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
    // Takes the filter off every microphone and restarts the audio
    // service; must run while kyrr.exe still exists.
    if not Exec(ExpandConstant('{app}\kyrr.exe'), '--setup uninstall', '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
      MsgBox(FmtMessage(CustomMessage('UninstallFailed'), [IntToStr(Code)]), mbError, MB_OK);
  end;
  if (CurUninstallStep = usPostUninstall) and RemoveData then
  begin
    DelTree(ExpandConstant('{localappdata}\kyrr'), True, True, True);
    DelTree(ExpandConstant('{userappdata}\kyrr'), True, True, True);
  end;
end;
