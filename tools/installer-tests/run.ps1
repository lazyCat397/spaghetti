<#
  Tests install.ps1 / uninstall.ps1 with PowerShell 7 (Linux or Windows).
  Windows-only pieces (registry, the `code` command, keyboard, cargo) are replaced by
  fakes; everything else (folders, rename/replace, copying, self-test through a real
  spg binary) runs for real.

    pwsh -File tools/installer-tests/run.ps1 -SpgBin target/release/spg [-Only name]
  Every check must also be proven by SABOTAGE (break install.ps1 on purpose, see it fail).
#>
param([Parameter(Mandatory)][string]$SpgBin, [string]$Only = '', [switch]$Show)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$SpgBin = (Resolve-Path $SpgBin).Path
$sandboxRoot = Join-Path ([IO.Path]::GetTempPath()) ('spg-insttest-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $sandboxRoot | Out-Null

# ---- load the installer (functions only) ----
. (Join-Path $repo 'install.ps1')
$ErrorActionPreference = 'Stop'

# ---- fakes ----
$script:UserPath = 'C:\Windows;%USERPROFILE%\bin'
$script:EnvVars = @{}
$script:Answers = New-Object System.Collections.Queue
$script:Prompts = @()
$script:CodePath = $null
$script:CargoOk = $true
$script:BuildOk = $true
$script:BuildCalls = 0
$script:CopyFail = 0

function Get-UserPathRaw { return $script:UserPath }
function Set-UserPathRaw([string]$Value) { $script:UserPath = $Value }
function Set-UserEnvVar([string]$Name, [string]$Value) { $script:EnvVars[$Name] = $Value }
function Read-Answer([string]$Prompt) {
    $script:Prompts += $Prompt
    if ($script:Answers.Count -eq 0) { throw "UNSCRIPTED PROMPT: $Prompt" }
    return [string]$script:Answers.Dequeue()
}
function Get-CodeCommand { return $script:CodePath }
function Get-CargoInfo { if ($script:CargoOk) { return @{ Cargo = 'cargo 1.99.0'; Rustc = 'rustc 1.99.0'; Host = 'x86_64-pc-windows-msvc' } } else { return $null } }
function Invoke-Build {
    $script:BuildCalls++
    if ($script:BuildOk) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $script:Fresh) | Out-Null
        Copy-Item -LiteralPath $SpgBin -Destination $script:Fresh -Force
        return @{ Ok = $true; Elapsed = [TimeSpan]::FromSeconds(1.5) }
    }
    return @{ Ok = $false; Elapsed = [TimeSpan]::FromSeconds(2.0) }
}

# ---- helpers ----
$script:Pass = 0; $script:Fail = 0; $script:Cur = ''
function Check($Cond, [string]$Msg) {
    if ($Cond) { $script:Pass++ } else { $script:Fail++; Write-Host "    FAIL [$($script:Cur)] $Msg" -ForegroundColor Red }
}
function New-Sandbox([string]$Name) {
    $d = Join-Path $sandboxRoot $Name
    New-Item -ItemType Directory -Path $d | Out-Null
    return $d
}
function New-FakeExe([string]$Path, [string]$Version, [string]$RunOutput) {
    # a tiny shell script posing as spg.exe
    $body = "#!/bin/bash`nif [ `"`$1`" = `"--version`" ]; then echo `"spg $Version`"; exit 0; fi`necho `"$RunOutput`"`n"
    Set-Content -LiteralPath $Path -Value $body -NoNewline
    chmod +x $Path
}
function New-FakeCode([string]$Dir, [string]$InstalledVersion) {
    $log = Join-Path $Dir 'code.log'
    $p = Join-Path $Dir 'code'
    $s = "#!/bin/bash`necho `"`$@`" >> '$log'`ncase `"`$1`" in`n  --list-extensions) $(if ($InstalledVersion) { "echo 'spaghetti.spaghetti@$InstalledVersion'" } else { ':' }) ;;`n  --install-extension) exit 0 ;;`nesac`n"
    Set-Content -LiteralPath $p -Value $s -NoNewline
    chmod +x $p
    return @{ Path = $p; Log = $log }
}
function New-FakeRoot([string]$Name) {
    $r = Join-Path $sandboxRoot ("root-" + $Name + "-" + [Guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Force -Path (Join-Path $r 'examples') | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'Cargo.toml') -Destination $r
    Get-ChildItem -LiteralPath (Join-Path $repo 'examples') -Filter '*.spg' -File | ForEach-Object { Microsoft.PowerShell.Management\Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $r 'examples') }
    return $r
}
function Reset-State([string]$Name) {
    $script:Cur = $Name
    $script:Root = New-FakeRoot $Name   # the installer only ever sees this fake source folder
    $script:UserPath = 'C:\Windows;%USERPROFILE%\bin'
    $script:EnvVars = @{}
    $script:Answers = New-Object System.Collections.Queue
    $script:Prompts = @()
    $script:CodePath = $null
    $script:CargoOk = $true; $script:BuildOk = $true; $script:BuildCalls = 0
    $script:Yes = [switch]$false; $script:NoBuild = [switch]$false
    $script:InstallDir = ''; $script:Exe = ''; $script:Vsix = ''
}
function Answer([string[]]$A) { $script:AnswersLog = $A | ForEach-Object { if ($_ -eq '') { '<Enter>' } else { $_ } }; foreach ($x in $A) { $script:Answers.Enqueue($x) } }
function Run-Installer {
    $out = @()
    try { $out = @(Invoke-Installer 6>&1) } catch { $out = @($_.Exception.Message) }
    $rc = ($out | Where-Object { $_ -is [int] } | Select-Object -Last 1)
    $text = (@($out | Where-Object { $_ -isnot [int] } | ForEach-Object { "$_" }) -join "`n")
    if ($Show) {
        Write-Host "`n===== transcript [$($script:Cur)] (answers typed: $(($script:AnswersLog) -join ' | ')) ====="
        Write-Host $text
        Write-Host "===== rc=$rc, prompts asked: $($script:Prompts.Count) ====="
    }
    return @{ Rc = $rc; Text = $text }
}
function Sandbox-Exe([string]$Dir) {
    # the "freshly built" exe the installer will pick up
    $script:Fresh = Join-Path $Dir 'fresh\spg.exe'
    $script:Exe = $script:Fresh
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $script:Fresh) | Out-Null
    Copy-Item -LiteralPath $SpgBin -Destination $script:Fresh -Force
    chmod +x $script:Fresh
}
function Want([string]$Name) { return ($Only -eq '' -or $Only -eq $Name) }

# =====================================================================
if (Want 'fresh-unattended') { try {
    Reset-State 'fresh-unattended'
    $sb = New-Sandbox 'fresh'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $script:Yes = [switch]$true; $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    $r = Run-Installer
    Check ($r.Rc -eq 0) "exit code 0 (got $($r.Rc))`n$($r.Text)"
    Check (Test-Path (Join-Path $dest 'spg.exe')) 'spg.exe installed'
    Check (@(Get-ChildItem (Join-Path $dest "examples") -Filter *.spg).Count -gt 5) 'examples copied'
    Check ($script:EnvVars['SPG_HOME'] -eq $dest) 'SPG_HOME set'
    Check ($script:UserPath -eq "C:\Windows;%USERPROFILE%\bin;$dest") "PATH appended with %VAR% entry preserved raw (got '$($script:UserPath)')"
    Check ($r.Text -match 'Self-test passed') 'self-test ran and passed'
    Check ($script:Prompts.Count -eq 0) 'no prompts in -Yes mode'
    Check ($r.Text -match 'Spaghetti is ready') 'success banner'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'interactive-fresh') { try {
    Reset-State 'interactive-fresh'
    $sb = New-Sandbox 'ifresh'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $script:NoBuild = [switch]$true
    $fc = New-FakeCode $sb ''; $script:CodePath = $fc.Path
    $script:Vsix = Join-Path $sb 'spaghetti-9.9.9.vsix'; Set-Content $script:Vsix 'x'
    # folder, path?, extension?, examples?, selftest?, go?
    Answer @($dest, '', '', 'n', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check (-not (Test-Path (Join-Path $dest 'examples'))) 'examples skipped when answered n'
    Check ((Get-Content $fc.Log -Raw) -match '--install-extension .*spaghetti-9\.9\.9\.vsix --force') 'extension installed via code with the vsix'
    Check ($r.Text -match 'Install the VSCode extension|Plan') 'plan shown'
    Check ($script:UserPath.EndsWith($dest)) 'PATH updated'
    Check ($script:Answers.Count -eq 0) 'all scripted answers consumed (no missing question)'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-rename') { try {
    Reset-State 'foreign-rename'
    $sb = New-Sandbox 'frn'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null
    Set-Content (Join-Path $dest 'mamma.txt') 'receita'
    New-Item -ItemType Directory -Path (Join-Path $dest 'molho') | Out-Null
    Set-Content (Join-Path $dest 'molho\tomate.txt') 'tomate'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    # replace? n -> new name (default) -> path, examples, selftest, go
    Answer @('n', '', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($r.Text -match 'does not look like a Spaghetti install') 'collision explained'
    Check ($r.Text -match 'mamma\.txt') 'contents of the colliding folder listed'
    Check (Test-Path (Join-Path $sb 'spaghetti-old\mamma.txt')) 'old folder renamed to spaghetti-old with content intact'
    Check ((Get-Content (Join-Path $sb 'spaghetti-old\molho\tomate.txt')) -eq 'tomate') 'nested content intact'
    Check (Test-Path (Join-Path $dest 'spg.exe')) 'new install in the original path'
    Check (-not (Test-Path (Join-Path $dest 'mamma.txt'))) 'old file is not in the new folder'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-rename-custom-and-clash') { try {
    Reset-State 'foreign-rename-custom-and-clash'
    $sb = New-Sandbox 'frc'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'a.txt') 'a'
    New-Item -ItemType Directory -Path (Join-Path $sb 'taken') | Out-Null
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    # replace? n -> name 'taken' (clash) -> 'bad/name' (invalid) -> 'guardada' ok
    Answer @('n', 'taken', 'bad/name', 'guardada', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($r.Text -match "'taken' already exists") 'clashing name rejected'
    Check ($r.Text -match 'cannot contain') 'invalid name rejected'
    Check (Test-Path (Join-Path $sb 'guardada\a.txt')) 'renamed to the accepted name'
    Check (Test-Path (Join-Path $sb 'taken')) 'the clashing folder was not touched'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-replace') { try {
    Reset-State 'foreign-replace'
    $sb = New-Sandbox 'frp'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'lixo.txt') 'x'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('y', 'YES', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check (-not (Test-Path (Join-Path $dest 'lixo.txt'))) 'old content deleted'
    Check (-not (Test-Path (Join-Path $sb 'spaghetti-old'))) 'nothing renamed'
    Check (Test-Path (Join-Path $dest 'spg.exe')) 'new install present'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-replace-needs-capital-YES') { try {
    Reset-State 'foreign-replace-needs-capital-YES'
    $sb = New-Sandbox 'frp2'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'importante.txt') 'x'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    # y, but confirmation 'yes' (lowercase) is NOT enough -> falls to rename
    Answer @('y', 'yes', '', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check (Test-Path (Join-Path $sb 'spaghetti-old\importante.txt')) 'lowercase yes did not delete; item was renamed instead'
    Check ($r.Text -match 'Not confirmed') 'user told nothing was deleted'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-cancel') { try {
    Reset-State 'foreign-cancel'
    $sb = New-Sandbox 'fcn'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'x.txt') 'x'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('n', 'cancel')
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'cancel is a clean exit'
    Check ($r.Text -match 'Cancelled') 'says cancelled'
    Check (@(Get-ChildItem $dest).Count -eq 1 -and (Test-Path (Join-Path $dest 'x.txt'))) 'folder untouched'
    Check ($script:EnvVars.Count -eq 0 -and $script:UserPath -eq 'C:\Windows;%USERPROFILE%\bin') 'environment untouched'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'foreign-unattended-refuses') { try {
    Reset-State 'foreign-unattended-refuses'
    $sb = New-Sandbox 'fur'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'x.txt') 'x'
    $script:Yes = [switch]$true; $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    $r = Run-Installer
    Check ($r.Rc -ne 0) 'fails (non-zero) instead of touching a foreign folder'
    Check ($r.Text -match 'Refusing to touch') 'explains why'
    Check (@(Get-ChildItem $dest).Count -eq 1) 'folder untouched'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'name-is-a-file') { try {
    Reset-State 'name-is-a-file'
    $sb = New-Sandbox 'nif'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'spaghetti'
    Set-Content $dest 'sou um arquivo'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('n', '', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($r.Text -match 'it is a file, not a folder') 'recognised as a file'
    Check ((Get-Content (Join-Path $sb 'spaghetti-old')) -eq 'sou um arquivo') 'file renamed with content intact'
    Check ((Get-Item $dest).PSIsContainer) 'new install is a folder'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'existing-install-update') { try {
    Reset-State 'existing-install-update'
    $sb = New-Sandbox 'upd'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    New-Item -ItemType Directory -Path $dest | Out-Null
    New-FakeExe (Join-Path $dest 'spg.exe') '0.0.1' 'old'
    Set-Content (Join-Path $dest 'notas-minhas.txt') 'minhas notas'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('', '', '', '', '', '')   # update?, path, examples, selftest, go (+spare)
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($r.Text -match 'Spaghetti 0\.0\.1 is installed') 'old version detected and shown'
    Check ($script:Prompts[0] -match 'Update it to 0\.1\.0\?') 'first question is the update question naming the new version'
    Check ((Get-Content (Join-Path $dest 'notas-minhas.txt')) -eq 'minhas notas') "user's own file in the folder survives an update"
    Check (-not (Test-Path (Join-Path $sb 'inst-old'))) 'an update is not a rename'
    $v = & (Join-Path $dest 'spg.exe') --version
    Check ($v -match '^spg 0\.14\.0') "new exe in place (got '$v')"
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'existing-install-decline') { try {
    Reset-State 'existing-install-decline'
    $sb = New-Sandbox 'dec'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    New-Item -ItemType Directory -Path $dest | Out-Null
    New-FakeExe (Join-Path $dest 'spg.exe') '0.0.1' 'old'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('n')
    $r = Run-Installer
    Check ($r.Rc -eq 0 -and $r.Text -match 'Cancelled') 'declining the update cancels'
    Check ((& (Join-Path $dest 'spg.exe') --version) -eq 'spg 0.0.1') 'old exe untouched'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'leftovers-are-reused') { try {
    Reset-State 'leftovers-are-reused'
    $sb = New-Sandbox 'lft'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    New-Item -ItemType Directory -Path (Join-Path $dest 'examples') -Force | Out-Null
    Set-Content (Join-Path $dest 'examples\a.spg') 'print(1)'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($r.Text -match 'leftovers of an earlier') 'leftovers recognised'
    Check (-not ($script:Prompts | Where-Object { $_ -match 'Replace it' })) 'no replace/rename question for our own leftovers'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'empty-folder-used') { try {
    Reset-State 'empty-folder-used'
    $sb = New-Sandbox 'emp'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'; New-Item -ItemType Directory -Path $dest | Out-Null
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0 -and (Test-Path (Join-Path $dest 'spg.exe'))) 'empty folder is simply used'
    Check (-not ($script:Prompts | Where-Object { $_ -match 'Replace it' })) 'no collision question'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'build-failure-changes-nothing') { try {
    Reset-State 'build-failure-changes-nothing'
    $sb = New-Sandbox 'bf'
    $script:Fresh = Join-Path $sb 'target\release\spg.exe'; $script:Exe = $script:Fresh
    $dest = Join-Path $sb 'spaghetti'
    New-Item -ItemType Directory -Path $dest | Out-Null; Set-Content (Join-Path $dest 'x.txt') 'x'
    $script:BuildOk = $false; $script:InstallDir = $dest
    # replace? n -> rename default; path, ext?, examples, selftest, cache?, go
    Answer @('n', '', '', '', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 1) "rc 1 on build failure (got $($r.Rc))"
    Check ($r.Text -match 'build failed') 'failure reported'
    Check ((Test-Path (Join-Path $dest 'x.txt')) -and -not (Test-Path (Join-Path $sb 'spaghetti-old'))) 'the colliding folder was NOT renamed because the build failed first'
    Check ($script:EnvVars.Count -eq 0 -and $script:UserPath -eq 'C:\Windows;%USERPROFILE%\bin') 'environment untouched'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'cargo-missing') { try {
    Reset-State 'cargo-missing'
    $sb = New-Sandbox 'cm'; $script:Fresh = Join-Path $sb 'target\release\spg.exe'; $script:Exe = $script:Fresh
    $script:CargoOk = $false; $script:Yes = [switch]$true; $script:InstallDir = (Join-Path $sb 'inst')
    $r = Run-Installer
    Check ($r.Rc -eq 1) 'rc 1'
    Check ($r.Text -match 'rustup\.rs') 'tells where to get Rust'
    Check (-not (Test-Path (Join-Path $sb 'inst'))) 'nothing created'
    Check ($script:BuildCalls -eq 0) 'no build attempted'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'build-then-install') { try {
    Reset-State 'build-then-install'
    $sb = New-Sandbox 'bti'
    $script:Fresh = Join-Path $sb 'target\release\spg.exe'; $script:Exe = $script:Fresh
    $dest = Join-Path $sb 'inst'
    $script:Yes = [switch]$true; $script:InstallDir = $dest
    $r = Run-Installer
    Check ($r.Rc -eq 0) "rc 0`n$($r.Text)"
    Check ($script:BuildCalls -eq 1) 'built exactly once'
    Check ($r.Text -match 'Built in 1\.5s') 'build time shown'
    Check (Test-Path (Join-Path $dest 'spg.exe')) 'installed'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'path-already-present') { try {
    Reset-State 'path-already-present'
    $sb = New-Sandbox 'pap'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $sep = [string][IO.Path]::DirectorySeparatorChar; $script:UserPath = "C:\Windows;$dest$sep;%USERPROFILE%\bin"
    $script:Yes = [switch]$true; $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'rc 0'
    Check ($script:UserPath -eq "C:\Windows;$dest$sep;%USERPROFILE%\bin") "PATH not duplicated nor rewritten (got '$($script:UserPath)')"
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'path-declined') { try {
    Reset-State 'path-declined'
    $sb = New-Sandbox 'pad'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('n', '', '', '')   # path n, examples, selftest, go
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'rc 0'
    Check ($script:UserPath -eq 'C:\Windows;%USERPROFILE%\bin') 'PATH untouched when declined'
    Check ($r.Text -match 'Run it as') 'tells how to run without PATH'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'extension-update') { try {
    Reset-State 'extension-update'
    $sb = New-Sandbox 'exu'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $fc = New-FakeCode $sb '0.1.0'; $script:CodePath = $fc.Path
    $script:Vsix = Join-Path $sb 'spaghetti-0.2.0.vsix'; Set-Content $script:Vsix 'x'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('', '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'rc 0'
    Check ($script:Prompts | Where-Object { $_ -match 'Update the VSCode extension \(0\.1\.0 -> 0\.2\.0\)' }) 'question names both versions'
    Check ((Get-Content $fc.Log -Raw) -match '--install-extension') 'extension installed'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'extension-declined') { try {
    Reset-State 'extension-declined'
    $sb = New-Sandbox 'exd'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $fc = New-FakeCode $sb ''; $script:CodePath = $fc.Path
    $script:Vsix = Join-Path $sb 'spaghetti-0.2.0.vsix'; Set-Content $script:Vsix 'x'
    $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    Answer @('', 'n', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'rc 0'
    Check (-not ((Get-Content $fc.Log -Raw) -match '--install-extension')) 'extension NOT installed when declined'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'selftest-failure') { try {
    Reset-State 'selftest-failure'
    $sb = New-Sandbox 'stf'
    $script:Fresh = Join-Path $sb 'fresh\spg.exe'; $script:Exe = $script:Fresh
    New-Item -ItemType Directory -Force -Path (Split-Path $script:Fresh) | Out-Null
    New-FakeExe $script:Fresh '0.1.0' 'garbage'
    $script:Yes = [switch]$true; $script:NoBuild = [switch]$true; $script:InstallDir = (Join-Path $sb 'inst')
    $r = Run-Installer
    Check ($r.Rc -eq 1) 'rc 1 when the self-test fails'
    Check ($r.Text -match 'Self-test failed') 'reported'
    Check (-not ($r.Text -match 'Spaghetti is ready')) 'no success banner'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'invalid-dirs') { try {
    Reset-State 'invalid-dirs'
    $cases = @(
        @{ Dir = ''; Re = 'empty' },
        @{ Dir = 'relativo\spg'; Re = 'full path' },
        @{ Dir = '/'; Re = 'not a whole drive' },
        @{ Dir = $script:Root; Re = 'source folder' },
        @{ Dir = (Split-Path -Parent $script:Root); Re = 'contains this source folder' },
        @{ Dir = (Join-Path $script:Root 'sub'); Re = 'inside this source folder' },
        @{ Dir = [Environment]::GetFolderPath('UserProfile'); Re = 'system or personal folder' },
        @{ Dir = "/tmp/a`0b"; Re = 'characters Windows does not allow|not a valid' }
    )
    foreach ($c in $cases) {
        $m = Test-InstallDirChoice $c.Dir
        Check ($null -ne $m -and $m -match $c.Re) "rejects '$($c.Dir)' with /$($c.Re)/ (got '$m')"
    }
    Check ($null -eq (Test-InstallDirChoice (Join-Path $sandboxRoot 'ok-dir'))) 'accepts a normal dedicated folder'
    # unattended: an invalid -InstallDir is fatal
    $script:Yes = [switch]$true; $script:InstallDir = $script:Root
    $r = Run-Installer
    Check ($r.Rc -ne 0 -and $r.Text -match 'source folder') 'unattended + bad folder = error, nothing happens'
    # interactive: a bad answer is re-asked
    Reset-State 'invalid-dirs-reask'
    $sb = New-Sandbox 'ida'; Sandbox-Exe $sb
    $script:NoBuild = [switch]$true
    Answer @($script:Root, (Join-Path $sb 'good'), '', '', '', '')
    $r = Run-Installer
    Check ($r.Rc -eq 0 -and (Test-Path (Join-Path $sb 'good\spg.exe'))) "re-asks after a bad folder`n$($r.Text)"
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'locked-exe-moved-aside') { try {
    Reset-State 'locked-exe-moved-aside'
    $sb = New-Sandbox 'lck'
    $src = Join-Path $sb 'src.exe'; Set-Content $src 'NEW'
    $dst = Join-Path $sb 'spg.exe'; Set-Content $dst 'OLD'
    $script:CopyFail = 1
    function Copy-Item { param([string]$LiteralPath, [string]$Destination, [switch]$Force, $ErrorAction)
        if ($script:CopyFail -gt 0) { $script:CopyFail--; throw 'The process cannot access the file because it is being used by another process.' }
        Microsoft.PowerShell.Management\Copy-Item -LiteralPath $LiteralPath -Destination $Destination -Force
    }
    $how = Install-Exe $src $dst
    Remove-Item Function:\Copy-Item
    Check ($how -eq 'moved-aside') "reports moved-aside (got $how)"
    Check ((Get-Content $dst) -eq 'NEW') 'new exe installed'
    Check ((Get-Content ($dst + '.old')) -eq 'OLD') 'old exe kept as spg.exe.old'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

if (Want 'cache-delete') { try {
    foreach ($case in @('y', 'n')) {
        Reset-State "cache-delete-$case"
        $sb = New-Sandbox "cache$case"
        # a fake source folder, so the REAL target\ of this repo can never be touched
        $fakeRoot = Join-Path $sb 'src'
        New-Item -ItemType Directory -Force -Path (Join-Path $fakeRoot 'target\release'), (Join-Path $fakeRoot 'examples') | Out-Null
        Set-Content (Join-Path $fakeRoot 'Cargo.toml') "[workspace.package]`nversion = `"9.9.9`"`n"
        Set-Content (Join-Path $fakeRoot 'examples\hello.spg') 'print("oi")'
        Set-Content (Join-Path $fakeRoot 'target\junk.bin') ('x' * 5000)
        $realRoot = $script:Root; $script:Root = $fakeRoot
        $script:Fresh = Join-Path $fakeRoot 'target\release\spg.exe'
        $dest = Join-Path $sb 'inst'; $script:InstallDir = $dest
        # path?, examples?, selftest?, delete cache?, go?
        Answer @('', '', '', $case, '')
        try { $r = Run-Installer } finally { $script:Root = $realRoot }
        Check ($r.Rc -eq 0) "[$case] rc 0`n$($r.Text)"
        Check (Test-Path (Join-Path $dest 'spg.exe')) "[$case] installed"
        Check ($r.Text -match 'Spaghetti 9\.9\.9') "[$case] version read from the source Cargo.toml"
        if ($case -eq 'y') { Check (-not (Test-Path (Join-Path $fakeRoot 'target'))) 'cache deleted when answered y' }
        else { Check (Test-Path (Join-Path $fakeRoot 'target\junk.bin')) 'cache kept when answered n' }
        Check (Test-Path (Join-Path $repo 'target\release')) 'the real target folder of the repo was not touched'
    }
} catch { Check $false ("exception: " + $_.Exception.Message) } }

# ---- uninstall ----
function Run-Uninstaller([string]$Dir) {
    $out = @()
    try { $out = @(Invoke-Uninstaller 6>&1) } catch { $out = @($_.Exception.Message) }
    $rc = ($out | Where-Object { $_ -is [int] } | Select-Object -Last 1)
    return @{ Rc = $rc; Text = (@($out | Where-Object { $_ -isnot [int] } | ForEach-Object { "$_" }) -join "`n") }
}
if (Want 'uninstall') { try {
    $self = $PSCommandPath
    # run the real uninstall.ps1 functions in a child scope with the same fakes
    Reset-State 'uninstall'
    $sb = New-Sandbox 'un'; Sandbox-Exe $sb
    $dest = Join-Path $sb 'inst'
    $script:Yes = [switch]$true; $script:NoBuild = [switch]$true; $script:InstallDir = $dest
    $r = Run-Installer
    Check ($r.Rc -eq 0) 'setup: installed'
    Set-Content (Join-Path $dest 'meu-arquivo.txt') 'meu'
    $pathAfterInstall = $script:UserPath

    . (Join-Path $repo 'uninstall.ps1') -InstallDir $dest -Yes
    function Get-UserPathRaw { return $script:UserPath }
    function Set-UserPathRaw([string]$Value) { $script:UserPath = $Value }
    function Set-UserEnvVar([string]$Name, $Value) { if ($null -eq $Value) { $script:EnvVars.Remove($Name) } else { $script:EnvVars[$Name] = $Value } }
    function Get-SpgHome { return $script:EnvVars['SPG_HOME'] }
    $u = Run-Uninstaller $dest
    Check ($u.Rc -eq 0) "uninstall rc 0`n$($u.Text)"
    Check (-not (Test-Path (Join-Path $dest 'spg.exe'))) 'spg.exe removed'
    Check (-not (Test-Path (Join-Path $dest 'examples'))) 'examples removed'
    Check ($script:UserPath -eq 'C:\Windows;%USERPROFILE%\bin') "PATH restored exactly, %VAR% entry intact (got '$($script:UserPath)')"
    Check (-not $script:EnvVars.ContainsKey('SPG_HOME')) 'SPG_HOME removed'
    Check ((Get-Content (Join-Path $dest 'meu-arquivo.txt')) -eq 'meu') "user's own file NOT deleted"
    Check ($u.Text -match 'was kept') 'folder kept because it still has the user file'
    Remove-Item (Join-Path $dest 'meu-arquivo.txt')
    $u2 = Run-Uninstaller $dest
    Check ($u2.Text -match 'Nothing to remove') 'second run: nothing to remove'
} catch { Check $false ("exception: " + $_.Exception.Message) } }

Write-Host ''
Write-Host ("installer tests: {0} checks passed, {1} failed" -f $script:Pass, $script:Fail) -ForegroundColor $(if ($script:Fail) { 'Red' } else { 'Green' })
Remove-Item -Recurse -Force $sandboxRoot -ErrorAction SilentlyContinue
exit $(if ($script:Fail) { 1 } else { 0 })
