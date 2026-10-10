; Instalador do Spaghetti (Inno Setup 6). Não exige administrador.
; O workflow copia spg.exe e spaghetti.vsix para installer\payload\ antes de compilar.
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppName=Spaghetti
AppVersion={#MyAppVersion}
AppPublisher=lazyCat397
DefaultDirName={localappdata}\Spaghetti
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
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath"; Description: "Adicionar o spg ao PATH (recomendado)"; GroupDescription: "Opções:"
Name: "assoc"; Description: "Associar arquivos .spg ao Spaghetti"; GroupDescription: "Opções:"
Name: "vscode"; Description: "Instalar a extensão do VSCode (se ele estiver instalado)"; GroupDescription: "Opções:"
Name: "desktopicon"; Description: "Criar atalho na área de trabalho"; GroupDescription: "Opções:"; Flags: unchecked

[Files]
Source: "payload\spg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\spaghetti.vsix"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Spaghetti (terminal)"; Filename: "{cmd}"; Parameters: "/k spg --help"
Name: "{autodesktop}\Spaghetti"; Filename: "{cmd}"; Parameters: "/k spg --help"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))
Root: HKCU; Subkey: "Software\Classes\.spg"; ValueType: string; ValueName: ""; ValueData: "SpaghettiFile"; Tasks: assoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SpaghettiFile"; ValueType: string; ValueName: ""; ValueData: "Arquivo Spaghetti"; Tasks: assoc; Flags: uninsdeletekey
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
