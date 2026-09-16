$ErrorActionPreference = 'Stop'
$crateName = 'epub3-kindle'
$version = (Select-String -LiteralPath 'Cargo.toml' -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
$directorySeparator = [string][System.IO.Path]::DirectorySeparatorChar
$forbidden = 'tools/screenshot-fixture/', 'sovereign_stars_vol_1.epub', 'sovereign_stars_vol_1_cover.png', 'sovereign_stars_vol_1_page_001.png', 'sovereign_stars_vol_1_page_002.png', 'sovereign_stars_vol_1.txt', 'target/'
$required = @(
    'README.md',
    'README.ja.md',
    'CHANGELOG.md',
    'LICENSE',
    'THIRDPARTY_LICENSES.md'
)

$contents = cargo package --list --allow-dirty --locked
$normalizedContents = @($contents | ForEach-Object { $_.Replace('\', '/').TrimStart('./').ToLowerInvariant() })
foreach ($entry in $contents) {
    $normalized = $entry.Replace('\', '/').ToLowerInvariant()
    if ($forbidden | Where-Object { $normalized.Contains($_) }) {
        throw "forbidden package entry: $entry"
    }
}
foreach ($requiredEntry in $required) {
    $normalizedRequired = $requiredEntry.ToLowerInvariant()
    if ($normalizedContents -notcontains $normalizedRequired) {
        throw "required public package entry missing: $requiredEntry"
    }
}

cargo package --allow-dirty --locked
$crate = Join-Path 'target/package' "$crateName-$version.crate"
if (-not (Test-Path -LiteralPath $crate)) { throw "crate archive not found: $crate" }
$root = Join-Path 'target' 'package-verify'
if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
New-Item -ItemType Directory -Path $root | Out-Null
tar -xf $crate -C $root
$extracted = Join-Path $root "$crateName-$version"
if (-not (Test-Path -LiteralPath $extracted)) { throw "extracted crate not found: $extracted" }
$extractedRoot = [System.IO.Path]::GetFullPath($extracted)
foreach ($readmeName in @('README.md', 'README.ja.md')) {
    $readmePath = Join-Path $extracted $readmeName
    $markdown = Get-Content -Raw -LiteralPath $readmePath
    $links = [regex]::Matches($markdown, '\]\(([^)\s]+)') |
        ForEach-Object { $_.Groups[1].Value } |
        Select-Object -Unique
    foreach ($link in $links) {
        if ($link.StartsWith('#')) {
            continue
        }
        $linkPath = $link.Split('#')[0].Replace('/', $directorySeparator).Replace('\', $directorySeparator)
        if ($link -match '^https?://') {
            continue
        }
        if ($link -match '^[A-Za-z][A-Za-z0-9+.-]*:') {
            throw "README link uses an unsupported URI scheme: $readmeName -> $link"
        }
        if ([System.IO.Path]::IsPathRooted($linkPath)) {
            throw "README link uses an absolute filesystem path: $readmeName -> $link"
        }
        $resolved = [System.IO.Path]::GetFullPath($linkPath, $extractedRoot)
        $relative = [System.IO.Path]::GetRelativePath($extractedRoot, $resolved)
        $parentPrefix = "..$directorySeparator"
        if ($relative -eq '..' -or $relative.StartsWith($parentPrefix, [System.StringComparison]::Ordinal)) {
            throw "README link escapes extracted crate: $readmeName -> $link"
        }
        if ($linkPath.StartsWith("docs$directorySeparator", [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw "README link does not resolve in extracted crate: $readmeName -> $link"
        }
    }
}
Push-Location $extracted
try { cargo test --all-features --locked } finally { Pop-Location }
Write-Output "Package verification passed: required public files are included, non-doc README links resolve, no screenshot fixture/assets or generated outputs are included, and extracted tests passed."
