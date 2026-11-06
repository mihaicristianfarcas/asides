# Packages asides for Windows: the app (asides-app.exe), its overlay and window (asides-overlay.exe)
# and the `asides` command (asides.exe), built for this computer's architecture, as a zip and an
# installer in dist\.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package-windows.ps1 [-Arch arm64|x64] [-Sign]
#     -> dist\asides-VERSION-windows-ARCH.zip        (the three programs, at the archive's root)
#        dist\asides-VERSION-windows-ARCH-setup.exe  (packaging\windows\asides.iss: per user, no
#                                                     administrator, in %LOCALAPPDATA%\Programs\asides)
#   ... -StageOnly [-Arch arm64|x64]       builds into target\stage\asides-VERSION-windows-ARCH, and stops
#   ... -From DIR -Arch arm64|x64 [-Sign]  packages what DIR holds, built elsewhere
#
# ARCH is arm64 or x64, this computer's unless -Arch says (x64 builds on Arm64 with MSVC's x64
# cross compiler, and runs there under emulation), VERSION the workspace's. The programs carry the C and C++ runtimes
# (crt-static, and whisper.cpp built with /MT): nothing needs installing beside them. The first
# time the app runs, it puts its folder on the user's PATH (the `asides` command), registers
# asides:// links and asks for nothing else.
#
# Needs Rust's MSVC toolchain, Visual Studio's C++ build tools (with clang-cl on Arm64, where
# whisper.cpp's ggml refuses MSVC), CMake, LLVM (libclang for whisper.cpp's bindings; clang for
# ring on Arm64), Ninja on Arm64, and Inno Setup 6 (ISCC.exe where it installs, or
# INNO_SETUP_COMPILER).
#
# -Sign signs the three programs before they're packaged, so the zip and the installer hold the
# same signed files, then the installer and its uninstaller (asides.iss's SignTool), with ssign:
# Certum's certificate in its cloud (SimplySign), from CERTUM_EMAIL and CERTUM_OTP (the account's
# TOTP seed) or CERTUM_TOKEN (a code read from the authenticator, for signing by hand), as ssign
# reads them. ssign is the one on PATH, or SSIGN. Each signature is timestamped (ssign's default,
# Certum's timestamp authority), so it stays valid after the certificate expires. The release
# workflow (.github/workflows/release.yml) builds with -StageOnly on each architecture's runner and
# signs and packages on another, with -From, as helio's does. Without -Sign nothing is signed, and
# SmartScreen asks before asides first starts.
#
# ASIDES_PROFILE=dev packages a debug build, its installer uncompressed (CI checks the packaging so).
param(
    [ValidateSet('arm64', 'x64')][string]$Arch,
    [switch]$StageOnly,
    [string]$From,
    [switch]$Sign
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content (Join-Path $repo 'Cargo.toml') -Raw
if ($manifest -notmatch '(?m)^version = "([^"]+)"') { throw 'no version in Cargo.toml' }
$version = $Matches[1]
# Not $profile: PowerShell's own name for the user's profile script.
$cargoProfile = if ($env:ASIDES_PROFILE) { $env:ASIDES_PROFILE } else { 'release' }
$host_arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'ARM64' { 'arm64' }
    'AMD64' { 'x64' }
    default { throw "asides isn't built on $env:PROCESSOR_ARCHITECTURE" }
}
if ($StageOnly -and $From) { throw 'Pass -StageOnly or -From, not both.' }
if ($From -and -not $Arch) { throw '-From needs -Arch: what the staged programs are built for.' }
$arch = if ($Arch) { $Arch } else { $host_arch }
$target = if ($arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$name = "asides-$version-windows-$arch"
$dist = Join-Path $repo 'dist'
$programs = 'asides-app.exe', 'asides-overlay.exe', 'asides.exe'

# The three programs and the licenses, built for $arch, in $stage.
function Build([string]$stage) {
    $llvm = 'C:\Program Files\LLVM\bin'
    if (-not $env:LIBCLANG_PATH -and (Test-Path $llvm)) { $env:LIBCLANG_PATH = $llvm }
    if ((Test-Path $llvm) -and ($env:Path -notlike "*$llvm*")) { $env:Path += ";$llvm" }
    # The C runtime in each program: Rust's, and whisper.cpp's through CMake (its CMAKE_ variables
    # reach CMake; the policy makes CMake 3.15's runtime setting count in its older project).
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $env:CMAKE_POLICY_DEFAULT_CMP0091 = 'NEW'
    $env:CMAKE_MSVC_RUNTIME_LIBRARY = 'MultiThreaded'
    if ($arch -eq 'arm64') {
        # ggml refuses MSVC on Arm64: clang-cl, with Ninja (Visual Studio's generator ignores it).
        $env:CMAKE_GENERATOR_aarch64_pc_windows_msvc = 'Ninja'
        $env:CC_aarch64_pc_windows_msvc = 'clang-cl'
        $env:CXX_aarch64_pc_windows_msvc = 'clang-cl'
        $env:CXXFLAGS_aarch64_pc_windows_msvc = '/EHsc'
    } elseif ($host_arch -eq 'arm64') {
        # x64 on Arm64: Visual Studio's generator looks for x64-hosted tools; Ninja takes the cross
        # compiler cc finds.
        $env:CMAKE_GENERATOR_x86_64_pc_windows_msvc = 'Ninja'
    }
    if ($arch -ne $host_arch) { rustup target add $target | Out-Null }

    Push-Location $repo
    try {
        # cargo's progress on stderr is not an error (Windows PowerShell makes it one under 'Stop'). A
        # target directory of its own: other flags would rebuild everything in target\ each time.
        $ErrorActionPreference = 'Continue'
        cargo build --profile $cargoProfile --target $target --target-dir target\windows-package -p asides-app -p asides-overlay -p asides-cli
        $ErrorActionPreference = 'Stop'
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    $out = if ($cargoProfile -eq 'dev') { 'debug' } else { $cargoProfile }
    $built = Join-Path $repo "target\windows-package\$target\$out"

    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    foreach ($program in $programs) {
        Copy-Item (Join-Path $built $program) $stage
    }
    foreach ($license in 'LICENSE-APACHE', 'LICENSE-MIT') {
        Copy-Item (Join-Path $repo $license) (Join-Path $stage "$license.txt")
    }
}

function Ssign {
    $ssign = if ($env:SSIGN) { $env:SSIGN } else { (Get-Command ssign -ErrorAction SilentlyContinue).Source }
    if (-not $ssign) { throw '-Sign needs ssign (cargo install ssign), on PATH or in SSIGN.' }
    if (-not $env:CERTUM_EMAIL -or -not ($env:CERTUM_OTP -or $env:CERTUM_TOKEN)) {
        throw '-Sign needs CERTUM_EMAIL, and CERTUM_OTP or CERTUM_TOKEN, in the environment (never on the command line).'
    }
    $ssign
}

# Signs the three programs, named as Windows shows them. One login signs them all: ssign keeps the
# session for 20 minutes, and the installer's signatures, a minute later, use it too.
function Sign-Programs([string]$ssign, [string]$stage) {
    $files = @($programs | ForEach-Object { Join-Path $stage $_ })
    & $ssign -n 'asides' -u 'https://asides.mihaicristian.dev' @files
    if ($LASTEXITCODE -ne 0) { throw "ssign failed ($LASTEXITCODE)" }
    $unsigned = @($files | Where-Object { (Get-AuthenticodeSignature $_).Status -ne 'Valid' })
    if ($unsigned.Count) { throw "not signed: $($unsigned -join ', ')" }
}

function Inno-Compiler {
    if ($env:INNO_SETUP_COMPILER) { return $env:INNO_SETUP_COMPILER }
    $found = Get-Command ISCC -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    foreach ($dir in ${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs')) {
        if (-not $dir) { continue }
        $iscc = Join-Path $dir 'Inno Setup 6\ISCC.exe'
        if (Test-Path $iscc) { return $iscc }
    }
    throw 'The installer needs Inno Setup 6 (https://jrsoftware.org/isinfo.php): ISCC.exe was not found (set INNO_SETUP_COMPILER to it).'
}

# The zip and the installer, from the files in $stage.
function Package([string]$stage, [string]$ssign) {
    New-Item -ItemType Directory -Force $dist | Out-Null
    $archive = Join-Path $dist "$name.zip"
    $setup = Join-Path $dist "$name-setup.exe"
    Remove-Item $archive, $setup -ErrorAction SilentlyContinue
    # The files at the archive's root, so Explorer's Extract All gives one folder.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($stage, $archive, 'Optimal', $false)

    $defines = @("/DVersion=$version", "/DArch=$arch", "/DSource=$stage", "/DOutput=$dist", "/DName=$name")
    if ($cargoProfile -eq 'dev') { $defines += '/DCompression=none' }
    if ($ssign) {
        # $q and $f are ISCC's: a quote, and the file to sign, quoted.
        $defines += '/DSign', ('/Sssign=$q' + $ssign + '$q -n $qasides$q -u $qhttps://asides.mihaicristian.dev$q $f')
    }
    # ISCC prints its progress on stdout; its errors are what matter.
    & (Inno-Compiler) /Q @defines (Join-Path $repo 'packaging\windows\asides.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
    if ($ssign -and (Get-AuthenticodeSignature $setup).Status -ne 'Valid') { throw "not signed: $setup" }

    foreach ($file in $archive, $setup) {
        $sum = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower()
        Write-Output "$sum  $(Split-Path -Leaf $file)"
    }
}

$ssign = if ($Sign -and -not $StageOnly) { Ssign } else { $null }
if ($From) {
    $stage = (Resolve-Path $From).Path
    foreach ($program in $programs) {
        if (-not (Test-Path (Join-Path $stage $program))) { throw "$From has no $program" }
    }
} else {
    $stage = Join-Path $repo "target\stage\$name"
    Build $stage
    if ($StageOnly) {
        Write-Output "staged $stage"
        return
    }
}
if ($ssign) { Sign-Programs $ssign $stage }
Package $stage $ssign
