$ErrorActionPreference = 'Stop'

$totalSteps = 20
$currentStep = 0
$script:FailureExitCode = 1
$locationPushed = $false

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Description,

        [Parameter(Mandatory)]
        [string]$FilePath,

        [Parameter()]
        [string[]]$Arguments = @()
    )

    $script:currentStep++
    Write-Host "[$script:currentStep/$totalSteps] $Description"

    $output = & $FilePath @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    foreach ($line in @($output)) {
        Write-Host $line
    }

    if ($exitCode -ne 0) {
        $script:FailureExitCode = $exitCode
        throw "$Description failed with exit code $exitCode."
    }

    return (@($output) -join [Environment]::NewLine)
}

try {
    $repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    Push-Location -LiteralPath $repoRoot
    $locationPushed = $true

    $packageVersionMatch = Select-String -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($null -eq $packageVersionMatch) {
        throw 'Could not determine the package version from Cargo.toml.'
    }
    $packageVersion = $packageVersionMatch.Matches[0].Groups[1].Value

    $stableRustcVersion = (Invoke-CheckedCommand -Description 'rustc --version' -FilePath 'rustc' -Arguments @('--version')).Trim()
    [void](Invoke-CheckedCommand -Description 'cargo --version' -FilePath 'cargo' -Arguments @('--version'))
    [void](Invoke-CheckedCommand -Description 'rustfmt --version' -FilePath 'rustfmt' -Arguments @('--version'))
    [void](Invoke-CheckedCommand -Description 'cargo clippy --version' -FilePath 'cargo' -Arguments @('clippy', '--version'))

    $activeToolchain = Invoke-CheckedCommand -Description 'active-toolchain stable check' -FilePath 'rustup' -Arguments @('show', 'active-toolchain')
    if ($activeToolchain -notmatch '(?m)^\s*stable(?:[-\s(]|$)') {
        throw "The active Rust toolchain is not stable: $activeToolchain"
    }

    [void](Invoke-CheckedCommand -Description 'cargo fmt --all -- --check' -FilePath 'cargo' -Arguments @('fmt', '--all', '--', '--check'))
    [void](Invoke-CheckedCommand -Description 'cargo check --all-targets --all-features --locked' -FilePath 'cargo' -Arguments @('check', '--all-targets', '--all-features', '--locked'))
    [void](Invoke-CheckedCommand -Description 'cargo clippy --all-targets --all-features --locked -- -D warnings' -FilePath 'cargo' -Arguments @('clippy', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings'))
    [void](Invoke-CheckedCommand -Description 'cargo test --all-features --locked' -FilePath 'cargo' -Arguments @('test', '--all-features', '--locked'))

    $previousRustdocFlags = [Environment]::GetEnvironmentVariable('RUSTDOCFLAGS', 'Process')
    $hadRustdocFlags = Test-Path -LiteralPath 'Env:RUSTDOCFLAGS'
    try {
        $env:RUSTDOCFLAGS = '-D warnings'
        [void](Invoke-CheckedCommand -Description 'cargo doc --no-deps --locked' -FilePath 'cargo' -Arguments @('doc', '--no-deps', '--locked'))
    }
    finally {
        if ($hadRustdocFlags) {
            $env:RUSTDOCFLAGS = $previousRustdocFlags
        }
        else {
            Remove-Item -LiteralPath 'Env:RUSTDOCFLAGS' -ErrorAction SilentlyContinue
        }
    }

    [void](Invoke-CheckedCommand -Description 'pwsh -File scripts/verify-package.ps1' -FilePath 'pwsh' -Arguments @('-File', 'scripts/verify-package.ps1'))
    [void](Invoke-CheckedCommand -Description 'cargo publish --dry-run --locked' -FilePath 'cargo' -Arguments @('publish', '--dry-run', '--locked'))

    [void](Invoke-CheckedCommand -Description 'Windows cargo check --all-features --locked' -FilePath 'cargo' -Arguments @('check', '--all-features', '--locked'))
    [void](Invoke-CheckedCommand -Description 'Windows cargo test --all-features --locked' -FilePath 'cargo' -Arguments @('test', '--all-features', '--locked'))
    [void](Invoke-CheckedCommand -Description 'Windows cargo build --release --all-features --locked' -FilePath 'cargo' -Arguments @('build', '--release', '--all-features', '--locked'))

    $currentStep++
    Write-Host "[$currentStep/$totalSteps] Verify Windows release executable and version metadata"
    $exePath = Join-Path $repoRoot (Join-Path 'target' (Join-Path 'release' 'epub3-kindle.exe'))
    if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
        throw "Windows release executable not found: $exePath"
    }
    if ($IsWindows) {
        $versionInfo = (Get-Item -LiteralPath $exePath).VersionInfo
        if ($versionInfo.FileVersion -ne $packageVersion) {
            throw "FileVersion $($versionInfo.FileVersion) does not match Cargo.toml package version $packageVersion."
        }
        if ($versionInfo.ProductVersion -ne $packageVersion) {
            throw "ProductVersion $($versionInfo.ProductVersion) does not match Cargo.toml package version $packageVersion."
        }
    }
    else {
        Write-Host 'VersionInfo comparison skipped because this is not Windows.'
    }

    try {
        $msrvRustcVersion = (Invoke-CheckedCommand -Description 'rustc +1.85.1 --version' -FilePath 'rustc' -Arguments @('+1.85.1', '--version')).Trim()
    }
    catch {
        throw "Rust 1.85.1 is unavailable. Install it with 'rustup toolchain install 1.85.1' and rerun. $($_.Exception.Message)"
    }
    [void](Invoke-CheckedCommand -Description 'cargo +1.85.1 --version' -FilePath 'cargo' -Arguments @('+1.85.1', '--version'))
    [void](Invoke-CheckedCommand -Description 'cargo +1.85.1 check --all-targets --all-features --locked' -FilePath 'cargo' -Arguments @('+1.85.1', 'check', '--all-targets', '--all-features', '--locked'))
    [void](Invoke-CheckedCommand -Description 'cargo +1.85.1 test --all-targets --all-features --locked' -FilePath 'cargo' -Arguments @('+1.85.1', 'test', '--all-targets', '--all-features', '--locked'))

    Write-Output "stable rustc version: $stableRustcVersion"
    Write-Output "MSRV rustc version: $msrvRustcVersion"
    Write-Output "package version: $packageVersion"
    Write-Output 'LOCAL RELEASE VERIFICATION: PASS'
    exit 0
}
catch {
    Write-Output "LOCAL RELEASE VERIFICATION ERROR: $($_.Exception.Message)"
    Write-Output 'LOCAL RELEASE VERIFICATION: FAIL'
    exit $script:FailureExitCode
}
finally {
    if ($locationPushed) {
        Pop-Location
    }
}
