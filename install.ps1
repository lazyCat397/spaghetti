<#
  Spaghetti installer: checks your system, asks a few questions, builds spg.exe
  and installs it. Normally started by build.bat.

    build.bat                              interactive (recommended)
    build.bat -Yes                         accept every default, ask nothing
    build.bat -InstallDir D:\spaghetti     choose the folder up front
    powershell -ExecutionPolicy Bypass -File install.ps1 [-NoBuild] [-Exe path] [-Vsix path]

  What it does:
    1. checks Rust (cargo), VSCode and whatever is already installed
    2. asks what you want: install folder, PATH, VSCode extension, examples...
       If the install folder collides with something that is NOT a Spaghetti
       install, it asks before touching it: replace it, or rename it out of the way.
    3. builds spg.exe (cargo build --release)
    4. installs spg.exe, the user PATH + SPG_HOME (no administrator needed),
       the VSCode extension and the examples
    5. runs a quick self-test
  Nothing outside this source folder is changed until the build has succeeded.
#>
param(
    [string]$InstallDir = '',
    [switch]$Yes,        # accept every default and never ask (never replaces a folder it does not recognise)
    [switch]$NoBuild,    # skip cargo and install the existing executable
    [string]$Exe = '',   # default: target\release\spg.exe
    [string]$Vsix = ''   # default: the newest dist\*.vsix
)
Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$script:Root       = $PSScriptRoot
$script:ExeName    = 'spg.exe'
$script:DefaultDir = 'C:\spaghetti'
$script:ExtId      = 'spaghetti.spaghetti'

# ============================== output ==============================

function Write-Ok([string]$Text)   { Write-Host '  [ok]    ' -ForegroundColor Green -NoNewline; Write-Host $Text }
function Write-Skip([string]$Text) { Write-Host '  [skip]  ' -ForegroundColor DarkGray -NoNewline; Write-Host $Text -ForegroundColor DarkGray }
function Write-Warn([string]$Text) { Write-Host '  [warn]  ' -ForegroundColor Yellow -NoNewline; Write-Host $Text }
function Write-Fail([string]$Text) { Write-Host '  [error] ' -ForegroundColor Red -NoNewline; Write-Host $Text }
function Write-Info([string]$Text) { Write-Host ('          ' + $Text) -ForegroundColor Gray }

function Write-Step([int]$N, [int]$Total, [string]$Title) {
    Write-Host ''
    Write-Host "[$N/$Total] $Title" -ForegroundColor Cyan
    Write-Host ('-' * 64) -ForegroundColor DarkCyan
}

# a box with a fixed inner width, so the right border always lines up
function Write-Box([string[]]$Lines, [string]$Color) {
    $w = 46
    Write-Host ('  +' + ('-' * $w) + '+') -ForegroundColor $Color
    foreach ($l in $Lines) { Write-Host ('  |' + ('   ' + $l).PadRight($w) + '|') -ForegroundColor $Color }
    Write-Host ('  +' + ('-' * $w) + '+') -ForegroundColor $Color
}

function Write-Banner([string]$Version) {
    $v = if ($Version) { "version $Version" } else { '' }
    Write-Host ''
    Write-Box @('S P A G H E T T I     installer', $v) 'Yellow'
}

function Write-PlanRow([string]$Label, [string]$Value) {
    Write-Host ('  {0,-16}' -f $Label) -ForegroundColor DarkGray -NoNewline
    Write-Host $Value
}

function Format-Size([long]$Bytes) {
    if ($Bytes -ge 1GB) { return ('{0:N1} GB' -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ('{0:N1} MB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N0} KB' -f ($Bytes / 1KB)) }
    return "$Bytes B"
}

function Format-Duration([TimeSpan]$T) {
    if ($T.TotalMinutes -ge 1) { return ('{0}m {1:00}s' -f [int][Math]::Floor($T.TotalMinutes), $T.Seconds) }
    return ('{0:N1}s' -f $T.TotalSeconds)
}

# ============================== input ==============================
# Read-Answer is the only place that touches the keyboard (tests replace it).

function Read-Answer([string]$Prompt) {
    $a = Read-Host $Prompt
    if ($null -eq $a) { return '' }
    return [string]$a
}

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

function Ask-Text([string]$Question, [string]$Default) {
    if ($Yes) { return $Default }
    $shown = if ($Default) { " [$Default]" } else { '' }
    $a = (Read-Answer "  $Question$shown").Trim()
    if ($a -eq '') { return $Default }
    return $a
}

# ============================== paths ==============================

function Get-FullPath([string]$P) {
    $full = [IO.Path]::GetFullPath($P)
    $root = [IO.Path]::GetPathRoot($full)
    if ($full.Length -gt $root.Length) {
        $full = $full.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    }
    return $full
}

function Test-SamePath([string]$A, [string]$B) {
    try { return [string]::Equals((Get-FullPath $A), (Get-FullPath $B), [StringComparison]::OrdinalIgnoreCase) }
    catch { return $false }
}

# true when $Child is $Parent itself or lives somewhere below it
function Test-IsInside([string]$Child, [string]$Parent) {
    try {
        $c = Get-FullPath $Child
        $p = Get-FullPath $Parent
        if ([string]::Equals($c, $p, [StringComparison]::OrdinalIgnoreCase)) { return $true }
        $sep = [string][IO.Path]::DirectorySeparatorChar
        if (-not $p.EndsWith($sep)) { $p = $p + $sep }
        return $c.StartsWith($p, [StringComparison]::OrdinalIgnoreCase)
    } catch { return $false }
}

# folders that must never be used (or erased) as an install folder
function Get-ProtectedPaths {
    $list = @(
        $env:SystemRoot, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramData,
        $env:APPDATA, $env:LOCALAPPDATA,
        [Environment]::GetFolderPath('UserProfile'),
        [Environment]::GetFolderPath('Desktop'),
        [Environment]::GetFolderPath('MyDocuments')
    )
    return @($list | Where-Object { $_ })
}

# returns a message when the folder is unusable, $null when it is fine
function Test-InstallDirChoice([string]$Dir) {
    if ([string]::IsNullOrWhiteSpace($Dir)) { return 'The folder cannot be empty.' }
    if ($Dir.IndexOfAny([IO.Path]::GetInvalidPathChars()) -ge 0) { return 'That path contains characters Windows does not allow.' }
    if (-not [IO.Path]::IsPathRooted($Dir)) { return "Use a full path, for example C:\spaghetti (got '$Dir')." }
    try { $full = Get-FullPath $Dir } catch { return 'That is not a valid path.' }
    $root = [IO.Path]::GetPathRoot($full)
    if ([string]::Equals($full, $root, [StringComparison]::OrdinalIgnoreCase)) { return 'Pick a folder, not a whole drive.' }
    if (-not (Test-Path -LiteralPath $root)) { return "The drive '$root' does not exist." }
    foreach ($p in (Get-ProtectedPaths)) {
        if (Test-SamePath $full $p) { return "'$full' is a system or personal folder; choose a dedicated one such as C:\spaghetti." }
        if (Test-IsInside $p $full) { return "'$full' contains a system or personal folder ($p); choose a dedicated folder." }
    }
    if (Test-IsInside $script:Root $full) { return "That folder contains this source folder ($($script:Root)); installing there could erase it." }
    if (Test-IsInside $full $script:Root) { return 'That folder is inside this source folder; choose one outside it.' }
    return $null
}

function Get-FolderSize([string]$Dir) {
    if (-not (Test-Path -LiteralPath $Dir -PathType Container)) { return [long]0 }
    $m = Get-ChildItem -LiteralPath $Dir -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum
    return [long]$m.Sum
}

# ============================== environment (Windows registry) ==============================
# Raw access, no %VAR% expansion: `setx` would truncate at 1024 characters and .NET
# would expand every entry. Tests replace these three functions.

function Get-UserPathRaw {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
    try { return [string]$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } finally { $key.Close() }
}
function Set-UserPathRaw([string]$Value) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try { $key.SetValue('Path', $Value, 'ExpandString') } finally { $key.Close() }
}
# Going through .NET makes Windows notify running programs that the environment
# changed (writing the registry alone does not).
function Set-UserEnvVar([string]$Name, [string]$Value) {
    [Environment]::SetEnvironmentVariable($Name, $Value, 'User')
}

function Test-OnUserPath([string]$Dir) {
    foreach ($e in ((Get-UserPathRaw) -split ';')) {
        if ($e -and (Test-SamePath ([Environment]::ExpandEnvironmentVariables($e)) $Dir)) { return $true }
    }
    return $false
}

function Add-ToUserPath([string]$Dir) {
    $parts = @((Get-UserPathRaw) -split ';' | Where-Object { $_ -ne '' })
    Set-UserPathRaw (($parts + $Dir.TrimEnd('\', '/')) -join ';')
}

# ============================== tools ==============================

function Get-SourceVersion {
    $toml = Join-Path $script:Root 'Cargo.toml'
    if (Test-Path -LiteralPath $toml) {
        foreach ($line in (Get-Content -LiteralPath $toml)) {
            if ($line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
        }
    }
    return $null
}

# version of an spg executable ("spg 0.14.0" -> "0.14.0"), $null if it does not run
function Get-ExeVersion([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & $Path --version 2>&1 | Select-Object -First 1
        if ("$out" -match '^spg\s+(\S+)') { return $Matches[1] }
    } catch { } finally { $ErrorActionPreference = $prev }
    return $null
}

function Get-CargoInfo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { return $null }
    $r = @{ Cargo = 'cargo'; Rustc = 'rustc'; Host = '' }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $r.Cargo = "$(& cargo --version 2>&1 | Select-Object -First 1)"
        $r.Rustc = "$(& rustc --version 2>&1 | Select-Object -First 1)"
        foreach ($l in @(& rustc -vV 2>&1)) { if ("$l" -match '^host:\s*(\S+)') { $r.Host = $Matches[1] } }
    } catch { } finally { $ErrorActionPreference = $prev }
    return $r
}

# the VSCode command line tool: the first of these that exists
function Get-CodeCommand {
    foreach ($n in @('code', 'code-insiders')) {
        $c = Get-Command $n -ErrorAction SilentlyContinue
        if ($c) { return $c.Source }
    }
    return $null
}

function Get-InstalledExtVersion([string]$Code) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        foreach ($l in @(& $Code --list-extensions --show-versions 2>&1)) {
            if ("$l" -match ('^' + [regex]::Escape($script:ExtId) + '@(\S+)')) { return $Matches[1] }
        }
    } catch { } finally { $ErrorActionPreference = $prev }
    return $null
}

function Find-Vsix {
    if ($Vsix) { return $Vsix }
    $dist = Join-Path $script:Root 'dist'
    if (-not (Test-Path -LiteralPath $dist)) { return $null }
    $f = Get-ChildItem -LiteralPath $dist -Filter '*.vsix' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($f) { return $f.FullName }
    return $null
}

function Get-VsixVersion([string]$Path) {
    if ($Path -match 'spaghetti-(\d+\.\d+\.\d+\S*?)\.vsix$') { return $Matches[1] }
    return $null
}

# running spg processes that would lock the executable (usually VSCode's language server)
function Get-RunningSpg([string]$Dir) {
    $found = @()
    foreach ($p in @(Get-Process -Name spg -ErrorAction SilentlyContinue)) {
        $path = $null
        try { $path = $p.Path } catch { }
        if ((-not $path) -or (Test-IsInside $path $Dir)) { $found += $p }
    }
    return $found
}

function Stop-SpgProcesses($Procs) {
    foreach ($p in $Procs) {
        try { Stop-Process -Id $p.Id -Force -ErrorAction Stop }
        catch { Write-Warn "Could not stop process $($p.Id): $($_.Exception.Message)" }
    }
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline) {
        $alive = @($Procs | Where-Object { Get-Process -Id $_.Id -ErrorAction SilentlyContinue })
        if ($alive.Count -eq 0) { break }
        Start-Sleep -Milliseconds 200
    }
}

# ============================== install folder ==============================

# What is at $Dir?  Kind: Missing | File | Empty | Spaghetti | Foreign
function Get-DirState([string]$Dir) {
    $s = @{ Kind = 'Missing'; Items = @(); Version = $null; Leftovers = $false }
    if (-not (Test-Path -LiteralPath $Dir)) { return $s }
    if (Test-Path -LiteralPath $Dir -PathType Leaf) { $s.Kind = 'File'; return $s }
    $items = @(Get-ChildItem -LiteralPath $Dir -Force -ErrorAction SilentlyContinue)
    $s.Items = $items
    if ($items.Count -eq 0) { $s.Kind = 'Empty'; return $s }
    $exe = Join-Path $Dir $script:ExeName
    if (Test-Path -LiteralPath $exe -PathType Leaf) {
        $s.Kind = 'Spaghetti'
        $s.Version = Get-ExeVersion $exe
        return $s
    }
    # no spg.exe, but nothing besides what this installer itself leaves behind
    $ours = @('examples', ($script:ExeName + '.old'))
    $strangers = @($items | Where-Object { $ours -notcontains $_.Name.ToLower() })
    if ($strangers.Count -eq 0) { $s.Kind = 'Spaghetti'; $s.Leftovers = $true; return $s }
    $s.Kind = 'Foreign'
    return $s
}

function Show-FolderContents([string]$Dir, $State) {
    if ($State.Kind -eq 'File') {
        $f = Get-Item -LiteralPath $Dir -Force
        Write-Info "It is a FILE (size $(Format-Size $f.Length), modified $($f.LastWriteTime.ToString('yyyy-MM-dd HH:mm')))."
        return
    }
    $items = @($State.Items)
    $size = Get-FolderSize $Dir
    Write-Info "It contains $($items.Count) item(s), $(Format-Size $size) in total:"
    foreach ($i in ($items | Select-Object -First 8)) {
        $what = if ($i.PSIsContainer) { 'folder' } else { Format-Size $i.Length }
        Write-Host ('            {0,-34} {1}' -f $i.Name, $what) -ForegroundColor Gray
    }
    if ($items.Count -gt 8) { Write-Info "... and $($items.Count - 8) more" }
}

function Get-FreeSiblingName([string]$Dir) {
    $parent = [IO.Path]::GetDirectoryName($Dir)
    $leaf = [IO.Path]::GetFileName($Dir)
    $name = "$leaf-old"
    $n = 2
    while (Test-Path -LiteralPath (Join-Path $parent $name)) { $name = "$leaf-old-$n"; $n++ }
    return $name
}

# $null when the name is fine, otherwise the reason
function Test-NewName([string]$Dir, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Name)) { return 'The name cannot be empty.' }
    if ($Name -eq '.' -or $Name -eq '..') { return 'That is not a usable name.' }
    if ($Name.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0) { return 'A name cannot contain \ / : * ? " < > |' }
    $parent = [IO.Path]::GetDirectoryName($Dir)
    if (Test-Path -LiteralPath (Join-Path $parent $Name)) { return "'$Name' already exists next to it." }
    return $null
}

# Decides what to do with the chosen folder. Nothing is changed here, only asked.
# Action: create | use | update | replace | rename | cancel
function Resolve-InstallFolder([string]$Dir, [string]$NewVersion) {
    $plan = @{ Action = 'create'; RenameTo = ''; State = $null; OldVersion = $null; Note = 'new folder' }
    $state = Get-DirState $Dir
    $plan.State = $state
    $leaf = [IO.Path]::GetFileName($Dir)

    switch ($state.Kind) {
        'Missing' { return $plan }
        'Empty'   { $plan.Action = 'use'; $plan.Note = 'existing empty folder'; return $plan }
        'Spaghetti' {
            $plan.OldVersion = $state.Version
            if ($state.Leftovers) {
                $plan.Action = 'use'; $plan.Note = 'leftovers of an earlier install, no spg.exe in it'
                Write-Ok "$Dir holds leftovers of an earlier Spaghetti install (no spg.exe); it will be reused."
                return $plan
            }
            $plan.Action = 'update'
            if ($state.Version) {
                $plan.Note = "update $($state.Version) -> $NewVersion"
                if ($state.Version -eq $NewVersion) {
                    Write-Ok "Spaghetti $($state.Version) is already installed in $Dir (same version as this source)."
                    $q = 'Reinstall it anyway (rebuilt from this source)?'
                } else {
                    Write-Ok "Spaghetti $($state.Version) is installed in $Dir; this source is $NewVersion."
                    $q = "Update it to ${NewVersion}?"
                }
            } else {
                $plan.Note = 'replace a spg.exe that does not run'
                Write-Warn "$Dir has a spg.exe that does not run (damaged, or built for another system)."
                $q = 'Replace it with a fresh build?'
            }
            if (-not (Ask-YesNo $q $true)) { $plan.Action = 'cancel' }
            return $plan
        }
    }

    # Foreign folder or a plain FILE with that name: it collides and has no Spaghetti content
    if ($state.Kind -eq 'File') { Write-Warn "'$Dir' exists, but it is a file, not a folder." }
    else { Write-Warn "'$Dir' already exists and does not look like a Spaghetti install (there is no spg.exe in it)." }
    Show-FolderContents $Dir $state

    if ($Yes) { throw "Refusing to touch '$Dir' in unattended mode (-Yes). Run build.bat without -Yes, or pick another folder with -InstallDir." }

    Write-Host ''
    if (Ask-YesNo "Replace it? That DELETES '$Dir' and everything in it." $false) {
        $c = Read-Answer "  Type YES (capital letters) to confirm the deletion, anything else to keep it"
        if ($c -ceq 'YES') {
            $plan.Action = 'replace'; $plan.Note = 'replaces the existing item (deleted)'
            return $plan
        }
        Write-Info 'Not confirmed, nothing will be deleted.'
    }

    Write-Info 'Then the existing item has to move out of the way: it will be renamed.'
    $suggest = Get-FreeSiblingName $Dir
    while ($true) {
        $name = Ask-Text "New name for the existing '$leaf' (type 'cancel' to abort)" $suggest
        if ($name.ToLower() -eq 'cancel') { $plan.Action = 'cancel'; return $plan }
        $problem = Test-NewName $Dir $name
        if (-not $problem) { break }
        Write-Warn $problem
    }
    $plan.Action = 'rename'; $plan.RenameTo = $name
    $plan.Note = "existing '$leaf' is renamed to '$name' first"
    return $plan
}

function Read-InstallDir {
    $dir = $InstallDir
    while ($true) {
        if (-not $dir) { $dir = Ask-Text 'Install folder' $script:DefaultDir }
        $problem = Test-InstallDirChoice $dir
        if (-not $problem) { return (Get-FullPath $dir) }
        if ($Yes) { throw $problem }
        Write-Warn $problem
        $dir = ''
    }
}

# ============================== install actions ==============================

# Copies the exe. A running exe cannot be overwritten on Windows, but it CAN be
# renamed: if the copy fails, the old file is moved aside as spg.exe.old.
function Install-Exe([string]$Src, [string]$Dst) {
    try {
        Copy-Item -LiteralPath $Src -Destination $Dst -Force -ErrorAction Stop
        return 'copied'
    } catch {
        if (-not (Test-Path -LiteralPath $Dst)) { throw }
        $old = $Dst + '.old'
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force -ErrorAction Stop }
        Move-Item -LiteralPath $Dst -Destination $old -Force -ErrorAction Stop
        Copy-Item -LiteralPath $Src -Destination $Dst -Force -ErrorAction Stop
        return 'moved-aside'
    }
}

function Copy-Examples([string]$Dir) {
    $src = Join-Path $script:Root 'examples'
    $dst = Join-Path $Dir 'examples'
    if (-not (Test-Path -LiteralPath $src)) { return 0 }
    New-Item -ItemType Directory -Force -Path $dst | Out-Null
    $files = @(Get-ChildItem -LiteralPath $src -Filter '*.spg' -File)
    foreach ($f in $files) { Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $dst $f.Name) -Force }
    return $files.Count
}

function Install-Extension([string]$Code, [string]$VsixPath) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = @(& $Code --install-extension $VsixPath --force 2>&1)
        $rc = $LASTEXITCODE
    } finally { $ErrorActionPreference = $prev }
    return @{ Ok = ($rc -eq 0); Output = @($out | ForEach-Object { "$_" }) }
}

# runs a tiny program through the installed spg.exe
function Invoke-SelfTest([string]$ExePath) {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('spg-selftest-' + [Guid]::NewGuid().ToString('N') + '.spg')
    $expected = 'hello from spaghetti'
    Set-Content -LiteralPath $tmp -Value ('print("' + $expected + '")') -Encoding ASCII
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = @(& $ExePath $tmp 2>&1 | ForEach-Object { "$_" })
        $rc = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
        Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
    }
    $text = (($out -join "`n")).Trim()
    return @{ Ok = ($rc -eq 0 -and $text -eq $expected); Output = $text; ExitCode = $rc }
}

function Invoke-Build {
    Push-Location $script:Root
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        & cargo build --release
        $rc = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
        Pop-Location
    }
    $sw.Stop()
    return @{ Ok = ($rc -eq 0); Elapsed = $sw.Elapsed }
}

# ============================== the whole flow ==============================

function Invoke-Installer {
    $t0 = Get-Date
    $total = 5
    $newVersion = Get-SourceVersion
    Write-Banner $newVersion
    if ($Yes) { Write-Host '  Unattended mode (-Yes): every question gets its default answer.' -ForegroundColor DarkGray }

    # ---------------- 1. checks ----------------
    Write-Step 1 $total 'Checking your system'
    $exeSrc = if ($Exe) { $Exe } else { [IO.Path]::Combine($script:Root, 'target', 'release', $script:ExeName) }
    $vsixPath = Find-Vsix
    $vsixVersion = if ($vsixPath) { Get-VsixVersion $vsixPath } else { $null }

    Write-Ok "Source folder: $($script:Root)$(if ($newVersion) { "  (Spaghetti $newVersion)" })"

    if ($NoBuild) {
        if (-not (Test-Path -LiteralPath $exeSrc -PathType Leaf)) { throw "-NoBuild was given but '$exeSrc' does not exist." }
        Write-Skip "Rust: not needed (-NoBuild), using $exeSrc"
    } else {
        $cargo = Get-CargoInfo
        if (-not $cargo) {
            Write-Fail 'cargo (Rust) was not found.'
            Write-Info 'Spaghetti is built from source, so Rust is needed once, to compile it.'
            Write-Info '1. Install Rust from https://rustup.rs (choose the default options).'
            Write-Info '2. If it asks for the "Visual Studio C++ Build Tools", install them too.'
            Write-Info '3. Close this window, open a new one and run build.bat again.'
            if (-not $Yes) {
                if (Ask-YesNo 'Open https://rustup.rs in your browser now?' $true) { Start-Process 'https://rustup.rs' }
            }
            return 1
        }
        Write-Ok "Rust: $($cargo.Cargo); $($cargo.Rustc)"
        if ($cargo.Host) { Write-Info "build target: $($cargo.Host)" }
    }

    if (Test-Path -LiteralPath $exeSrc -PathType Leaf) {
        $f = Get-Item -LiteralPath $exeSrc
        Write-Info "previous build found: $(Format-Size $f.Length), built $($f.LastWriteTime.ToString('yyyy-MM-dd HH:mm'))"
    }

    $code = Get-CodeCommand
    $codeExtVersion = $null
    if ($code) {
        $codeExtVersion = Get-InstalledExtVersion $code
        $have = if ($codeExtVersion) { "Spaghetti extension $codeExtVersion is installed" } else { 'Spaghetti extension is not installed' }
        Write-Ok "VSCode: found ('$([IO.Path]::GetFileName($code))'); $have"
    } else {
        Write-Warn "VSCode: the 'code' command was not found (the extension can still be installed by hand)."
    }
    if ($vsixPath) { Write-Ok "Extension package: $([IO.Path]::GetFileName($vsixPath))" }
    else { Write-Warn 'No .vsix found in dist\ - the extension cannot be installed.' }

    # ---------------- 2. questions ----------------
    Write-Step 2 $total 'A few questions (Enter = the default in brackets)'
    $dir = Read-InstallDir
    $folder = Resolve-InstallFolder $dir $newVersion
    if ($folder.Action -eq 'cancel') {
        Write-Host ''
        Write-Host '  Cancelled. Nothing was changed.' -ForegroundColor Yellow
        return 0
    }

    # other spg.exe on the PATH?
    $others = @(Get-Command spg -All -ErrorAction SilentlyContinue | Where-Object { $_.CommandType -eq 'Application' } |
        ForEach-Object { $_.Source } | Where-Object { $_ -and -not (Test-IsInside $_ $dir) })
    foreach ($o in $others) { Write-Warn "Another spg was found on your PATH: $o (the first one found wins)." }

    # running spg processes would lock the file
    $procs = @()
    $stopProcs = $false
    if (Test-Path -LiteralPath $dir -PathType Container) {
        $procs = @(Get-RunningSpg $dir)
        if ($procs.Count -gt 0) {
            Write-Warn "spg is running ($($procs.Count) process(es), probably the VSCode language server)."
            $stopProcs = Ask-YesNo 'Stop it so the file can be replaced? (VSCode restarts it by itself)' $true
            if (-not $stopProcs) { Write-Info 'OK: the old file will be moved aside (spg.exe.old) instead of overwritten.' }
        }
    }

    $onPath = $false
    $addPath = $false
    if (Test-OnUserPath $dir) { $onPath = $true; Write-Ok "$dir is already on your PATH." }
    else { $addPath = Ask-YesNo "Add $dir to your PATH, so 'spg' works in any terminal?" $true }

    $doExt = $false
    if ($code -and $vsixPath) {
        $q = if ($codeExtVersion) { "Update the VSCode extension ($codeExtVersion -> $vsixVersion)?" } else { "Install the VSCode extension ($vsixVersion)?" }
        $doExt = Ask-YesNo $q $true
    }

    $exampleCount = @(Get-ChildItem -LiteralPath (Join-Path $script:Root 'examples') -Filter '*.spg' -File -ErrorAction SilentlyContinue).Count
    $doExamples = $false
    if ($exampleCount -gt 0) { $doExamples = Ask-YesNo "Copy the $exampleCount example programs to $(Join-Path $dir 'examples')?" $true }

    $doTest = Ask-YesNo 'Run a quick self-test when it is installed?' $true

    $targetDir = [IO.Path]::Combine($script:Root, 'target')
    $deleteTarget = $false
    if (-not $NoBuild) {
        $tsize = Get-FolderSize $targetDir
        if ($tsize -gt 0) {
            $deleteTarget = Ask-YesNo "Delete the build cache '$targetDir' afterwards ($(Format-Size $tsize))? Keeping it makes the next build much faster." $false
        }
    }

    # ---- the plan ----
    Write-Host ''
    Write-Host '  Plan' -ForegroundColor Cyan
    $verText = if ($newVersion) { $newVersion } else { 'this source' }
    Write-PlanRow 'Install folder' "$dir   ($($folder.Note))"
    Write-PlanRow 'spg.exe' $(if ($NoBuild) { "existing build, $verText" } else { "build from source, $verText" })
    Write-PlanRow 'PATH' $(if ($onPath) { 'already there' } elseif ($addPath) { "add $dir (user PATH, no admin needed)" } else { 'leave alone' })
    Write-PlanRow 'VSCode extension' $(if ($doExt) { "install $vsixVersion" } elseif ($code -and $vsixPath) { 'skip' } else { 'not available' })
    Write-PlanRow 'Examples' $(if ($doExamples) { "copy $exampleCount programs" } else { 'skip' })
    Write-PlanRow 'Self-test' $(if ($doTest) { 'yes' } else { 'no' })
    if ($deleteTarget) { Write-PlanRow 'Build cache' 'delete when done' }
    Write-Host ''
    if (-not (Ask-YesNo 'Go ahead?' $true)) {
        Write-Host '  Cancelled. Nothing was changed.' -ForegroundColor Yellow
        return 0
    }

    # ---------------- 3. build ----------------
    Write-Step 3 $total $(if ($NoBuild) { 'Building (skipped, -NoBuild)' } else { 'Building spg.exe (release)' })
    if ($NoBuild) {
        Write-Skip "using the existing $exeSrc"
    } else {
        Write-Info 'First build takes a minute or two; later ones are much faster. Output from cargo:'
        Write-Host ''
        $b = Invoke-Build
        Write-Host ''
        if (-not $b.Ok) {
            Write-Fail "The build failed after $(Format-Duration $b.Elapsed). Nothing was installed."
            Write-Info 'Read the first "error" lines above. Common causes:'
            Write-Info '- "linker link.exe not found": install the Visual Studio C++ Build Tools (the Rust installer offers them).'
            Write-Info '- an old Rust: run "rustup update".'
            return 1
        }
        Write-Ok "Built in $(Format-Duration $b.Elapsed)"
    }
    if (-not (Test-Path -LiteralPath $exeSrc -PathType Leaf)) { throw "The build finished but '$exeSrc' is missing." }
    $built = Get-Item -LiteralPath $exeSrc
    $builtVersion = Get-ExeVersion $exeSrc
    Write-Ok "$($script:ExeName): $(Format-Size $built.Length)$(if ($builtVersion) { ", version $builtVersion" })"
    if (-not $builtVersion) { Write-Warn 'The built exe did not answer --version; continuing anyway.' }

    # ---------------- 4. install ----------------
    Write-Step 4 $total 'Installing'
    $done = @()

    switch ($folder.Action) {
        'rename' {
            Rename-Item -LiteralPath $dir -NewName $folder.RenameTo -ErrorAction Stop
            Write-Ok "Existing '$dir' renamed to '$($folder.RenameTo)'"
        }
        'replace' {
            if ($stopProcs -and $procs.Count -gt 0) { Stop-SpgProcesses $procs; $procs = @() }
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction Stop
            Write-Ok "Deleted the existing '$dir'"
        }
    }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Write-Ok "Folder ready: $dir"

    if ($stopProcs -and $procs.Count -gt 0) {
        Stop-SpgProcesses $procs
        Write-Ok "Stopped $($procs.Count) running spg process(es)"
    }

    $exeDst = Join-Path $dir $script:ExeName
    $how = Install-Exe $exeSrc $exeDst
    if ($how -eq 'moved-aside') { Write-Ok "$($script:ExeName) installed (the running one was moved aside as $($script:ExeName).old; restart VSCode to use the new one)" }
    else { Write-Ok "$($script:ExeName) installed to $dir" }
    $done += "spg.exe -> $exeDst"

    if ($onPath) {
        Write-Skip 'PATH: already contains the install folder'
    } elseif ($addPath) {
        Add-ToUserPath $dir
        Write-Ok "$dir added to your user PATH"
        $done += 'user PATH updated'
    } else {
        Write-Skip "PATH: left alone. Run it as $exeDst"
    }
    Set-UserEnvVar 'SPG_HOME' $dir
    Write-Ok "SPG_HOME = $dir"

    if ($doExamples) {
        $n = Copy-Examples $dir
        Write-Ok "$n example program(s) copied to $(Join-Path $dir 'examples')"
        $done += "examples -> $(Join-Path $dir 'examples')"
    }

    if ($doExt) {
        $r = Install-Extension $code $vsixPath
        if ($r.Ok) {
            Write-Ok "VSCode extension $vsixVersion installed"
            Write-Info 'Already-open VSCode windows: Ctrl+Shift+P, then "Developer: Reload Window".'
            $done += "VSCode extension $vsixVersion"
        } else {
            Write-Warn 'The VSCode extension did not install:'
            foreach ($l in ($r.Output | Select-Object -First 4)) { Write-Info $l }
            Write-Info "By hand: VSCode > Extensions > ... > Install from VSIX > $vsixPath"
        }
    } elseif ($code -and $vsixPath) {
        Write-Skip 'VSCode extension: skipped'
    }

    if ($deleteTarget) {
        try {
            Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction Stop
            Write-Ok "Build cache deleted ($targetDir)"
        } catch { Write-Warn "Could not delete '$targetDir': $($_.Exception.Message)" }
    }

    # ---------------- 5. verify ----------------
    Write-Step 5 $total 'Verifying'
    $installedVersion = Get-ExeVersion $exeDst
    if ($installedVersion) { Write-Ok "$exeDst --version -> spg $installedVersion" }
    else { Write-Warn "$exeDst did not answer --version." }
    $testOk = $true
    if ($doTest) {
        $t = Invoke-SelfTest $exeDst
        if ($t.Ok) { Write-Ok "Self-test passed: a program printed ""$($t.Output)""" }
        else {
            $testOk = $false
            Write-Fail "Self-test failed (exit code $($t.ExitCode))."
            if ($t.Output) { Write-Info $t.Output }
        }
    } else { Write-Skip 'Self-test skipped' }

    # ---------------- summary ----------------
    $elapsed = (Get-Date) - $t0
    Write-Host ''
    if ($testOk -and $installedVersion) {
        Write-Box @('Spaghetti is ready.') 'Green'
    } else {
        Write-Host '  Installed, but the checks above did not all pass.' -ForegroundColor Yellow
    }
    foreach ($d in $done) { Write-Info $d }
    Write-Host ''
    Write-Host '  Next steps' -ForegroundColor Cyan
    if ($addPath) { Write-Info 'Open a NEW terminal (the PATH is only read when a terminal starts), then:' }
    else { Write-Info 'In a terminal:' }
    $cmd = if ($addPath -or $onPath) { 'spg' } else { """$exeDst""" }
    Write-Info "  $cmd --version"
    $sample = if ($doExamples) { Join-Path (Join-Path $dir 'examples') 'hello.spg' } else { 'your_program.spg' }
    Write-Info "  $cmd $sample"
    if ($doExt) { Write-Info 'In VSCode, open a .spg file and press Ctrl+F5 to run it. Type "@math." to see namespace members.' }
    Write-Info 'To remove everything later: powershell -ExecutionPolicy Bypass -File uninstall.ps1'
    Write-Host ''
    Write-Host "  Total time: $(Format-Duration $elapsed)" -ForegroundColor DarkGray
    if ($testOk -and $installedVersion) { return 0 } else { return 1 }
}

# Not run when the file is dot-sourced (the tests do that).
if ($MyInvocation.InvocationName -ne '.') {
    $rc = 1
    try { $rc = Invoke-Installer }
    catch {
        Write-Host ''
        Write-Fail $_.Exception.Message
        Write-Info 'The installation stopped. Run build.bat again after fixing the problem above.'
    }
    exit $rc
}
