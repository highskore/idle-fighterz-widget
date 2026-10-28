# Powerrr installer for Windows: irm https://highskore.com/install.ps1 | iex
# Downloads the latest release from github.com/highskore/powerrr, checks its sha256 against SHA256SUMS, runs the
# per-user installer silently (no admin; it goes to %LOCALAPPDATA%\Powerrr) and starts the app. A download made here
# carries no mark-of-the-web, so SmartScreen doesn't stop an unsigned build. POWERRR_REPO picks another repo, and
# POWERRR_RELEASE (a release's API URL) a release other than the latest: powerrr's CI points it at the build under test.
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'   # Windows PowerShell's progress bar slows downloads to a crawl
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  $repo = if ($env:POWERRR_REPO) { $env:POWERRR_REPO } else { 'highskore/powerrr' }
  if (-not [Environment]::Is64BitOperatingSystem) { throw 'install: there is only an x64 build' }

  $api = if ($env:POWERRR_RELEASE) { $env:POWERRR_RELEASE } else { "https://api.github.com/repos/$repo/releases/latest" }
  Write-Host "finding the release at $api..."
  $release = Invoke-RestMethod $api -Headers @{ 'User-Agent' = 'powerrr-install' }
  $setup = $release.assets | Where-Object name -like '*_x64-setup.exe' | Select-Object -First 1
  $sums = $release.assets | Where-Object name -eq 'SHA256SUMS' | Select-Object -First 1
  if (-not $setup -or -not $sums) { throw "install: release $($release.tag_name) has no Windows installer or SHA256SUMS" }

  $tmp = Join-Path ([IO.Path]::GetTempPath()) ("powerrr-" + [Guid]::NewGuid())
  New-Item -ItemType Directory $tmp | Out-Null
  try {
    $file = Join-Path $tmp $setup.name
    Write-Host "downloading $($setup.name)..."
    Invoke-WebRequest $setup.browser_download_url -OutFile $file -UseBasicParsing
    Invoke-WebRequest $sums.browser_download_url -OutFile "$tmp\SHA256SUMS" -UseBasicParsing
    $want = Get-Content "$tmp\SHA256SUMS" |
      ForEach-Object { $h, $n = $_.Trim() -split '\s+\*?', 2; if ($n -eq $setup.name) { $h } } | Select-Object -First 1
    $got = (Get-FileHash $file -Algorithm SHA256).Hash
    if (-not $want -or $want -ne $got) { throw "install: sha256 mismatch for $($setup.name) (expected $want, got $got)" }
    Write-Host 'sha256 ok'

    $dir = Join-Path $env:LOCALAPPDATA 'Powerrr'
    Get-Process | Where-Object { $_.Path -like "$dir\*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    $run = Start-Process $file -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "install: the installer exited with $($run.ExitCode)" }
    $exe = Get-ChildItem "$dir\*.exe" -ErrorAction SilentlyContinue | Where-Object Name -notlike 'uninstall*' | Select-Object -First 1
    if (-not $exe) { throw "install: the installer ran, but there's no app in $dir" }
    Write-Host "installed $($exe.FullName)"
    Start-Process $exe.FullName
    Write-Host "it's in the tray (the ^ by the clock if it's hidden), and Ctrl+Alt+P shows or hides it"
  } finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
  }
}
