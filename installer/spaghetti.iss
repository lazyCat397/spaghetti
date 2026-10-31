; Spaghetti installer (Inno Setup 6). Does not require administrator rights.
; The workflow copies spg.exe and spaghetti.vsix into installer\payload\ before compiling.
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{4A6E6493-9857-4957-956D-A7B70D8FF315}
AppName=Spaghetti
AppVersion={#MyAppVersion}
AppPublisher=lazyCat397
DefaultDirName={userprofile}\Spaghetti
DefaultGroupName=Spaghetti
OutputBaseFilename=spaghetti-{#MyAppVersion}
OutputDir=.
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
ChangesEnvironment=yes
WizardStyle=modern
UninstallDisplayName=Spaghetti {#MyAppVersion}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "addtopath"; Description: "Add spg to PATH (recommended)"; GroupDescription: "Options:"
Name: "assoc"; Description: "Associate .spg files with Spaghetti"; GroupDescription: "Options:"
Name: "vscode"; Description: "Install the VSCode extension (if VSCode is installed)"; GroupDescription: "Options:"
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Options:"; Flags: unchecked

[Files]
Source: "payload\spg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\spaghetti.vsix"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\stdlib\*"; DestDir: "{app}\stdlib"; Flags: ignoreversion recursesubdirs
Source: "payload\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs

[InstallDelete]
; the stdlib folder is ours: clear it so libraries removed in a newer version (fork.spg) don't stay behind
Type: filesandordirs; Name: "{app}\stdlib"
; the examples folder is ours too: a sample removed in a newer version must not stay behind
Type: filesandordirs; Name: "{app}\examples"

[Icons]
Name: "{group}\Spaghetti (terminal)"; Filename: "{cmd}"; Parameters: "/k spg --help"
Name: "{autodesktop}\Spaghetti"; Filename: "{cmd}"; Parameters: "/k spg --help"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))
Root: HKCU; Subkey: "Software\Classes\.spg"; ValueType: string; ValueName: ""; ValueData: "SpaghettiFile"; Tasks: assoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SpaghettiFile"; ValueType: string; ValueName: ""; ValueData: "Spaghetti file"; Tasks: assoc; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SpaghettiFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\spg.exe,0"; Tasks: assoc
Root: HKCU; Subkey: "Software\Classes\SpaghettiFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\spg.exe"" ""%1"""; Tasks: assoc

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  CodeCmd: string;
  ResultCode: Integer;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('vscode') then
  begin
    CodeCmd := ExpandConstant('{localappdata}\Programs\Microsoft VS Code\bin\code.cmd');
    if FileExists(CodeCmd) then
      Exec(CodeCmd, '--install-extension "' + ExpandConstant('{app}\spaghetti.vsix') + '" --force',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;
