<#
  Removes Spaghetti: spg.exe, the examples, the PATH entry, SPG_HOME and the VSCode
  extension. It asks first. Your own files in the install folder are never deleted
  (the folder itself is removed only when nothing else is left in it).

    powershell -ExecutionPolicy Bypass -File uninstall.ps1 [-InstallDir C:\spaghetti] [-Yes]
#>
param(
    [string]$InstallDir = '',
    [switch]$Yes
)
Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

function Write-Ok([string]$Text)   { Write-Host '  [ok]    ' -ForegroundColor Green -NoNewline; Write-Host $Text }
function Write-Skip([string]$Text) { Write-Host '  [skip]  ' -ForegroundColor DarkGray -NoNewline; Write-Host $Text -ForegroundColor DarkGray }
function Write-Warn([string]$Text) { Write-Host '  [warn]  ' -ForegroundColor Yellow -NoNewline; Write-Host $Text }
function Write-Info([string]$Text) { Write-Host ('          ' + $Text) -ForegroundColor Gray }

function Read-Answer([string]$Prompt) { $a = Read-Host $Prompt; if ($null -eq $a) { return '' }; return [string]$a }

function Ask-YesNo([string]$Question, [bool]$Default) {
    if ($Yes) { return $Default }
    $hint = if ($Default) { 'Y/n' } else { 'y/N' }
    while ($true) {
        $a = (Read-Answer "  $Question [$hint]").Trim().ToLower()
        if ($a -eq '') { return $Default }
        if ($a -eq 'y' -or $a -eq 'yes') { return $true }
        if ($a -eq 'n' -or $a -eq 'no')  { return $false }
        Write-Warn 'Please answer y or n.'
    }
}

function Same-Path([string]$A, [string]$B) {
    try {
        return [string]::Equals([IO.Path]::GetFullPath($A).TrimEnd('\', '/'), [IO.Path]::GetFullPath($B).TrimEnd('\', '/'), [StringComparison]::OrdinalIgnoreCase)
    } catch { return $false }
}

function Get-UserPathRaw {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
    try { return [string]$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } finally { $key.Close() }
}
function Set-UserPathRaw([string]$Value) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try { $key.SetValue('Path', $Value, 'ExpandString') } finally { $key.Close() }
}
function Get-SpgHome { return [Environment]::GetEnvironmentVariable('SPG_HOME', 'User') }
function Set-UserEnvVar([string]$Name, $Value) { [Environment]::SetEnvironmentVariable($Name, $Value, 'User') }

function Invoke-Uninstaller {
    Write-Host ''
    Write-Host '  Spaghetti uninstaller' -ForegroundColor Yellow
    Write-Host ('  ' + ('-' * 40)) -ForegroundColor DarkYellow

    # where is it installed? explicit > SPG_HOME > the default
    $dir = $InstallDir
    if (-not $dir) { $dir = Get-SpgHome }
    if (-not $dir) { $dir = 'C:\spaghetti' }
    $dir = [IO.Path]::GetFullPath($dir).TrimEnd('\', '/')

    $exe = Join-Path $dir 'spg.exe'
    $hasExe = Test-Path -LiteralPath $exe -PathType Leaf
    $pathParts = @((Get-UserPathRaw) -split ';' | Where-Object { $_ -ne '' })
    $onPath = @($pathParts | Where-Object { Same-Path ([Environment]::ExpandEnvironmentVariables($_)) $dir }).Count -gt 0
    $code = $null
    foreach ($n in @('code', 'code-insiders')) { $c = Get-Command $n -ErrorAction SilentlyContinue; if ($c) { $code = $c.Source; break } }
    $extInstalled = $false
    if ($code) {
        $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try { $extInstalled = @(& $code --list-extensions 2>&1 | Where-Object { "$_" -eq 'spaghetti.spaghetti' }).Count -gt 0 } finally { $ErrorActionPreference = $prev }
    }
    $spgHome = Get-SpgHome

    Write-Host ''
    Write-Host '  Found' -ForegroundColor Cyan
    Write-Info ("install folder   $dir" + $(if (Test-Path -LiteralPath $dir) { '' } else { '  (does not exist)' }))
    Write-Info ("spg.exe          " + $(if ($hasExe) { 'yes' } else { 'no' }))
    Write-Info ("on the PATH      " + $(if ($onPath) { 'yes' } else { 'no' }))
    Write-Info ("SPG_HOME         " + $(if ($spgHome) { $spgHome } else { 'not set' }))
    Write-Info ("VSCode extension " + $(if ($extInstalled) { 'installed' } elseif ($code) { 'not installed' } else { "VSCode 'code' command not found" }))

    if (-not ($hasExe -or $onPath -or $spgHome -or $extInstalled -or (Test-Path -LiteralPath $dir))) {
        Write-Host ''
        Write-Host '  Nothing to remove.' -ForegroundColor Yellow
        return 0
    }
    Write-Host ''
    if (-not (Ask-YesNo 'Remove all of this?' $true)) { Write-Host '  Cancelled. Nothing was changed.' -ForegroundColor Yellow; return 0 }
    Write-Host ''

    # running processes would lock the file
    foreach ($p in @(Get-Process -Name spg -ErrorAction SilentlyContinue)) {
        $pp = $null; try { $pp = $p.Path } catch { }
        if ($pp -and (Same-Path ([IO.Path]::GetDirectoryName($pp)) $dir)) {
            try { Stop-Process -Id $p.Id -Force -ErrorAction Stop; Write-Ok "Stopped running spg (process $($p.Id))" }
            catch { Write-Warn "Could not stop process $($p.Id): $($_.Exception.Message)" }
        }
    }

    if ($onPath) {
        $keep = @($pathParts | Where-Object { -not (Same-Path ([Environment]::ExpandEnvironmentVariables($_)) $dir) })
        Set-UserPathRaw ($keep -join ';')
        Write-Ok "$dir removed from your user PATH"
    } else { Write-Skip 'PATH: nothing to remove' }

    if ($spgHome) { Set-UserEnvVar 'SPG_HOME' $null; Write-Ok 'SPG_HOME removed' }

    foreach ($f in @('spg.exe', 'spg.exe.old')) {
        $p = Join-Path $dir $f
        if (Test-Path -LiteralPath $p -PathType Leaf) {
            try { Remove-Item -LiteralPath $p -Force -ErrorAction Stop; Write-Ok "$f deleted" }
            catch { Write-Warn "Could not delete ${f}: $($_.Exception.Message) (close programs using it and try again)" }
        }
    }
    $ex = Join-Path $dir 'examples'
    if (Test-Path -LiteralPath $ex -PathType Container) {
        Get-ChildItem -LiteralPath $ex -Filter '*.spg' -File | Remove-Item -Force -ErrorAction SilentlyContinue
        if (@(Get-ChildItem -LiteralPath $ex -Force).Count -eq 0) { Remove-Item -LiteralPath $ex -Force; Write-Ok 'examples deleted' }
        else { Write-Warn 'examples: the .spg files were deleted, but the folder has other files of yours and stays.' }
    }
    if (Test-Path -LiteralPath $dir -PathType Container) {
        if (@(Get-ChildItem -LiteralPath $dir -Force).Count -eq 0) { Remove-Item -LiteralPath $dir -Force; Write-Ok "$dir deleted (it was empty)" }
        else { Write-Warn "$dir still has files that are not Spaghetti's, so it was kept." }
    }

    if ($extInstalled) {
        $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try { & $code --uninstall-extension spaghetti.spaghetti 2>&1 | Out-Null; $rc = $LASTEXITCODE } finally { $ErrorActionPreference = $prev }
        if ($rc -eq 0) { Write-Ok 'VSCode extension removed' } else { Write-Warn 'Could not remove the VSCode extension (do it in the Extensions panel).' }
    }

    Write-Host ''
    Write-Host '  Spaghetti was removed. Open a new terminal for the PATH change to apply.' -ForegroundColor Green
    return 0
}

if ($MyInvocation.InvocationName -ne '.') {
    $rc = 1
    try { $rc = Invoke-Uninstaller }
    catch { Write-Host ''; Write-Host "  [error] $($_.Exception.Message)" -ForegroundColor Red }
    exit $rc
}
