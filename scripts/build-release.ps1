# build-release.ps1 — Build all binaries in release mode and produce a versioned
# distributable zip for Windows x86_64.
#
# Usage:
#   .\scripts\build-release.ps1
#
# Output:
#   dist\<name>-<version>-windows-x86_64.zip
#     api.exe
#     worker.exe
#     updater.exe
#     installer.exe
#     uninstaller.exe
#     config\default.yaml

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Read name & version from Cargo.toml
$cargoToml = Get-Content "Cargo.toml" -Raw
$cargoName    = [regex]::Match($cargoToml, '(?m)^name\s*=\s*"([^"]+)"').Groups[1].Value
$cargoVersion = [regex]::Match($cargoToml, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value

$archSuffix  = "windows-x86_64"
$bundleName  = "$cargoName-$cargoVersion-$archSuffix"
$distDir     = "dist\$bundleName"
$releaseDir  = "target\release"

# Code Signing (optional)
# Set ONE of the following environment variables before running this script:
#
#   $env:SIGN_CERT_THUMBPRINT   SHA-1 thumbprint of a certificate already in the
#                               Windows certificate store (recommended for EV/HSM).
#                               Example: $env:SIGN_CERT_THUMBPRINT = "AABB1122..."
#
#   $env:SIGN_CERT_PATH         Path to a .pfx file. Also set SIGN_CERT_PASSWORD
#                               if the file is password-protected.
#
# If neither variable is set, signing is skipped (suitable for dev builds).
$signtool = $null
if ($env:SIGN_CERT_THUMBPRINT -or $env:SIGN_CERT_PATH) {
    $signtoolCmd = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($signtoolCmd) {
        $signtool = $signtoolCmd.Source
    } else {
        $kitBin = "C:\Program Files (x86)\Windows Kits\10\bin"
        if (Test-Path $kitBin) {
            $found = Get-ChildItem $kitBin -Recurse -Filter "signtool.exe" -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match "x64" } |
                Sort-Object FullName -Descending |
                Select-Object -First 1
            if ($found) { $signtool = $found.FullName }
        }
    }
    if (-not $signtool) {
        Write-Error "signtool.exe not found. Install the Windows SDK or add it to PATH."
        exit 1
    }
    Write-Host "==> Code signing enabled  : $signtool"
}

function Invoke-Sign {
    param([string]$FilePath)
    if (-not $signtool) { return }

    $signArgs = @(
        "sign",
        "/fd", "sha256",
        "/tr", "http://timestamp.digicert.com",
        "/td", "sha256"
    )
    if ($env:SIGN_CERT_THUMBPRINT) {
        $signArgs += @("/sha1", $env:SIGN_CERT_THUMBPRINT)
    } else {
        $signArgs += @("/f", $env:SIGN_CERT_PATH)
        if ($env:SIGN_CERT_PASSWORD) {
            $signArgs += @("/p", $env:SIGN_CERT_PASSWORD)
        }
    }
    $signArgs += $FilePath

    $name = [System.IO.Path]::GetFileName($FilePath)
    Write-Host "    Signing   $name"
    & $signtool @signArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Error "signtool sign failed for $name (exit $LASTEXITCODE)"
        exit $LASTEXITCODE
    }

    & $signtool verify /pa /q $FilePath
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Signature verification failed for $name"
        exit $LASTEXITCODE
    }
    Write-Host "    Verified  $name"
}

Write-Host "==> Building $cargoName v$cargoVersion (windows)"

# Compile
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Assemble bundle
Write-Host "==> Assembling bundle in $distDir\"
if (Test-Path $distDir) { Remove-Item $distDir -Recurse -Force }
New-Item -ItemType Directory -Path "$distDir\config" | Out-Null

foreach ($bin in @("api.exe", "worker.exe", "updater.exe", "installer.exe", "uninstaller.exe")) {
    $src = Join-Path $releaseDir $bin
    if (Test-Path $src) {
        Copy-Item $src "$distDir\$bin"
        Invoke-Sign "$distDir\$bin"
        Write-Host "    $bin"
    } else {
        Write-Warning "    $src not found, skipping"
    }
}

Copy-Item "config\default.yaml" "$distDir\config\default.yaml"

# Zip
if (-not (Test-Path "dist")) { New-Item -ItemType Directory -Path "dist" | Out-Null }
$zipPath = "dist\$bundleName.zip"
Write-Host "==> Creating $zipPath"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Compress-Archive -Path $distDir -DestinationPath $zipPath

# Checksum
Write-Host "==> Computing SHA-256"
$hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
$shaLine = "$hash  $zipPath"
$shaFile = "$zipPath.sha256"
Set-Content $shaFile $shaLine
Write-Host $shaLine

# Manifests
$publishedAt   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$manifest = [ordered]@{
    version         = $cargoVersion
    release_notes   = ""
    published_at    = $publishedAt
    platform        = $archSuffix
    download_url    = "https://YOUR_CDN/releases/v$cargoVersion/$bundleName.zip"
    checksum_sha256 = $hash
    binary_name     = "api.exe"
    signed          = [bool]$signtool
}
$manifestJson  = ConvertTo-Json $manifest -Depth 3

$latestPath    = "dist\latest-windows.json"
$versionedPath = "dist\v$cargoVersion-windows.json"
Set-Content $latestPath    $manifestJson -Encoding UTF8
Set-Content $versionedPath $manifestJson -Encoding UTF8

Write-Host ""
Write-Host "Release bundle         : $zipPath"
Write-Host "Manifest (latest)      : $latestPath"
Write-Host "Manifest (versioned)   : $versionedPath"
Write-Host ""
Write-Host $manifestJson

