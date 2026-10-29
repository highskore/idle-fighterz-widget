# Powerrr installer for Windows: irm https://highskore.com/install.ps1 | iex
# Downloads the latest release from github.com/highskore/powerrr, checks its sha256 against SHA256SUMS, runs the
# per-user installer silently (no admin; it goes to %LOCALAPPDATA%\Powerrr) and starts the app. A download made here
# carries no mark-of-the-web, so SmartScreen doesn't stop an unsigned build. POWERRR_REPO picks another repo, and
# POWERRR_RELEASE (a release's API URL) a release other than the latest: powerrr's CI points it at the build under test.
# It refuses to run elevated (the app would install for, and run as, the admin) unless POWERRR_ALLOW_ADMIN=1.
#
# A release's SHA256SUMS is signed with minisign (SHA256SUMS.minisig) by the release workflow, and checked here with
# the Ed25519 verification below, so nothing needs installing. A signed release that doesn't verify is refused, and so
# is an unsigned one, except the releases from before signing, listed below with their SHA256SUMS pinned. Once builds
# are Authenticode-signed, the publisher below is checked too.
# This file is served by highskore.com and kept as a copy in powerrr's packaging/install.ps1: change both.
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'   # Windows PowerShell's progress bar slows downloads to a crawl
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

  # the minisign public key that signs SHA256SUMS (minisign.pub's second line)
  $MinisignPublicKey = 'RWQsYREKR+Av/TIhq7yrFboAERzsd4eoUxepZrikLPg8/hrtXm7MvY1C'
  # the releases from before SHA256SUMS was signed, each with the sha256 of its SHA256SUMS: only these install unsigned
  $LegacySums = @{ 'desktop-v0.1.1' = 'dc73874f6220409f6b9443337c6a8f77b38713eb8e4ecb9530908eeaaf735ffd' }
  # the Authenticode signer of the installer, once builds are signed: its certificate's thumbprint and/or subject
  # (e.g. 'CN=Your Name, O=Your Name, L=City, C=XX'). Empty: not checked
  $SignerThumbprint = ''
  $SignerSubject = ''

  $repo = if ($env:POWERRR_REPO) { $env:POWERRR_REPO } else { 'highskore/powerrr' }
  if (-not [Environment]::Is64BitOperatingSystem) { throw 'install: there is only an x64 build' }
  $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  if ($admin -and $env:POWERRR_ALLOW_ADMIN -ne '1') {
    throw 'install: this is an elevated (admin) PowerShell. Powerrr installs for your user only: run this from a normal PowerShell, or set $env:POWERRR_ALLOW_ADMIN = 1 to install for this admin account anyway'
  }

  # Ed25519 verification (RFC 8032): S < L, and [S]B - [k]A encodes to R, where k = SHA-512(R || A || message).
  # PowerShell names are case-insensitive, and an alias (md is mkdir) wins over a function: no local below may share a
  # name with $P, $L, $D or $I, and no function is named like an alias
  function Test-Ed25519([byte[]]$key, [byte[]]$sig, [byte[]]$msg) {
    $P = [Numerics.BigInteger]::Pow(2, 255) - 19
    $L = [Numerics.BigInteger]::Pow(2, 252) + [Numerics.BigInteger]::Parse('27742317777372353535851937790883648493')
    function Fp($a) { $r = [Numerics.BigInteger]::Remainder($a, $P); if ($r.Sign -lt 0) { $r + $P } else { $r } }
    function Inv($a) { [Numerics.BigInteger]::ModPow((Fp $a), $P - 2, $P) }
    function Le([byte[]]$b) { [Numerics.BigInteger]::new([byte[]]($b + [byte[]]@(0))) }   # little-endian, unsigned
    $D = Fp (([Numerics.BigInteger]-121665) * (Inv 121666))
    $I = [Numerics.BigInteger]::ModPow(2, ($P - 1) / 4, $P)
    function Add($p1, $p2) {
      $aa = Fp (($p1[1] - $p1[0]) * ($p2[1] - $p2[0])); $bb = Fp (($p1[1] + $p1[0]) * ($p2[1] + $p2[0]))
      $cc = Fp ($D * 2 * $p1[3] * $p2[3]); $dd = Fp ($p1[2] * 2 * $p2[2])
      $e = $bb - $aa; $f = $dd - $cc; $g = $dd + $cc; $h = $bb + $aa
      , @((Fp ($e * $f)), (Fp ($g * $h)), (Fp ($f * $g)), (Fp ($e * $h)))
    }
    function Mul($s, $q) {
      $r = @([Numerics.BigInteger]::Zero, [Numerics.BigInteger]::One, [Numerics.BigInteger]::One, [Numerics.BigInteger]::Zero)
      while ($s.Sign -gt 0) { if (-not $s.IsEven) { $r = Add $r $q }; $q = Add $q $q; $s = $s / 2 }
      , $r
    }
    function Point([byte[]]$b) {
      $y = Le $b; $sign = [int]($b[31] -shr 7); $y = $y % [Numerics.BigInteger]::Pow(2, 255)
      if ($y -ge $P) { return $null }
      $x2 = Fp (($y * $y - 1) * (Inv ($D * $y * $y + 1)))
      $x = [Numerics.BigInteger]::ModPow($x2, ($P + 3) / 8, $P)
      if (-not (Fp ($x * $x - $x2)).IsZero) { $x = Fp ($x * $I) }
      if (-not (Fp ($x * $x - $x2)).IsZero) { return $null }
      if ($x.IsZero -and $sign -eq 1) { return $null }
      $odd = if ($x.IsEven) { 0 } else { 1 }
      if ($odd -ne $sign) { $x = $P - $x }
      , @($x, $y, [Numerics.BigInteger]::One, (Fp ($x * $y)))
    }
    function Encode($pt) {
      $z = Inv $pt[2]; $x = Fp ($pt[0] * $z); $y = Fp ($pt[1] * $z)
      if (-not $x.IsEven) { $y = $y + [Numerics.BigInteger]::Pow(2, 255) }
      $b = $y.ToByteArray(); [byte[]]$out = New-Object byte[] 32; [Array]::Copy($b, $out, [Math]::Min(32, $b.Length)); , $out
    }
    $A = Point $key
    $S = Le $sig[32..63]
    if ($null -eq $A -or $S -ge $L) { return $false }
    $sha = [Security.Cryptography.SHA512]::Create()
    $k = (Le $sha.ComputeHash([byte[]]($sig[0..31] + $key + $msg))) % $L
    $B = Point ([byte[]](@(0x58) + @(0x66) * 31))
    $minusA = @((Fp ($P - $A[0])), $A[1], $A[2], (Fp ($P - $A[3])))
    $R = Encode (Add (Mul $S $B) (Mul $k $minusA))
    return [BitConverter]::ToString($R) -eq [BitConverter]::ToString([byte[]]$sig[0..31])
  }

  # minisign's legacy format (minisign -l): Ed25519 over the file, then over the signature and its trusted comment.
  # Returns the trusted comment it verified, or $null
  function Test-Minisign([byte[]]$file, [byte[]]$minisig, [string]$publicKey) {
    $key = [Convert]::FromBase64String($publicKey)
    # exactly four lines (CRLF or LF), no bare CR and no NUL: the lines checked are the lines verified
    $text = [Text.Encoding]::UTF8.GetString($minisig).Replace("`r`n", "`n")
    $lines = $text -split "`n"
    if ($lines.Count -eq 5 -and $lines[4] -eq '') { $lines = $lines[0..3] }
    if ($text.IndexOf([char]13) -ge 0 -or $text.IndexOf([char]0) -ge 0 -or $lines.Count -ne 4 -or -not $lines[2].StartsWith('trusted comment: ')) { Write-Host 'SHA256SUMS.minisig is malformed'; return $null }
    $line = [Convert]::FromBase64String($lines[1]); $global = [Convert]::FromBase64String($lines[3])
    if ($key.Length -ne 42 -or $line.Length -ne 74 -or $global.Length -ne 64) { return $null }
    if ([Text.Encoding]::ASCII.GetString($line, 0, 2) -ne 'Ed') { Write-Host "SHA256SUMS.minisig isn't in minisign's legacy format (minisign -l)"; return $null }
    if ([BitConverter]::ToString($line, 2, 8) -ne [BitConverter]::ToString($key, 2, 8)) { Write-Host 'SHA256SUMS is signed with another key'; return $null }
    $pk = [byte[]]$key[10..41]; $sig = [byte[]]$line[10..73]
    $comment = $lines[2].Substring('trusted comment: '.Length)
    if ((Test-Ed25519 $pk $sig $file) -and (Test-Ed25519 $pk $global ([byte[]]($sig + [Text.Encoding]::UTF8.GetBytes($comment))))) { return $comment }
    return $null
  }

  $api = if ($env:POWERRR_RELEASE) { $env:POWERRR_RELEASE } else { "https://api.github.com/repos/$repo/releases/latest" }
  Write-Host "finding the release at $api..."
  $release = Invoke-RestMethod $api -Headers @{ 'User-Agent' = 'powerrr-install' }
  $setup = $release.assets | Where-Object name -like '*_x64-setup.exe' | Select-Object -First 1
  $sums = $release.assets | Where-Object name -eq 'SHA256SUMS' | Select-Object -First 1
  $minisig = $release.assets | Where-Object name -eq 'SHA256SUMS.minisig' | Select-Object -First 1
  if (-not $setup -or -not $sums) { throw "install: release $($release.tag_name) has no Windows installer or SHA256SUMS" }

  $tmp = Join-Path ([IO.Path]::GetTempPath()) ("powerrr-" + [Guid]::NewGuid())
  New-Item -ItemType Directory $tmp | Out-Null
  try {
    $file = Join-Path $tmp $setup.name
    Write-Host "downloading $($setup.name)..."
    Invoke-WebRequest $setup.browser_download_url -OutFile $file -UseBasicParsing
    Invoke-WebRequest $sums.browser_download_url -OutFile "$tmp\SHA256SUMS" -UseBasicParsing
    if ($minisig) {
      if (-not $MinisignPublicKey) { throw "install: this release's SHA256SUMS is signed, but this installer has no key to check it with: not installing" }
      Invoke-WebRequest $minisig.browser_download_url -OutFile "$tmp\SHA256SUMS.minisig" -UseBasicParsing
      $comment = Test-Minisign ([IO.File]::ReadAllBytes("$tmp\SHA256SUMS")) ([IO.File]::ReadAllBytes("$tmp\SHA256SUMS.minisig")) $MinisignPublicKey
      if ($null -eq $comment) { throw "install: the signature on SHA256SUMS doesn't verify: not installing" }
      # the trusted comment just verified names the release it was made for: an older release's SHA256SUMS and
      # signature, served as a newer one, are refused
      if ($comment -cne "powerrr $($release.tag_name)") { throw "install: SHA256SUMS was signed for '$comment', not 'powerrr $($release.tag_name)': not installing" }
      Write-Host "SHA256SUMS signature ok ($($release.tag_name))"
    } else {
      # without this, deleting the .minisig would turn any release back into an unsigned one
      $tag = [string]$release.tag_name
      if (-not $LegacySums.ContainsKey($tag)) { throw "install: release $tag has no SHA256SUMS.minisig, though releases are signed: not installing" }
      if ((Get-FileHash "$tmp\SHA256SUMS" -Algorithm SHA256).Hash -ne $LegacySums[$tag]) { throw "install: SHA256SUMS of $tag isn't the one it was released with: not installing" }
      Write-Host "note: unsigned checksums: $tag is from before releases were signed; its SHA256SUMS matches the copy pinned in this installer"
    }
    $want = Get-Content "$tmp\SHA256SUMS" |
      ForEach-Object { $h, $n = $_.Trim() -split '\s+\*?', 2; if ($n -eq $setup.name) { $h } } | Select-Object -First 1
    $got = (Get-FileHash $file -Algorithm SHA256).Hash
    if (-not $want -or $want -ne $got) { throw "install: sha256 mismatch for $($setup.name) (expected $want, got $got)" }
    Write-Host 'sha256 ok'

    $auth = Get-AuthenticodeSignature $file
    if ($SignerThumbprint -or $SignerSubject) {
      if ($auth.Status -ne 'Valid') { throw "install: $($setup.name) isn't validly signed ($($auth.Status)): not installing" }
      if ($SignerThumbprint -and $auth.SignerCertificate.Thumbprint -ne $SignerThumbprint) { throw "install: $($setup.name) is signed by $($auth.SignerCertificate.Thumbprint), not $SignerThumbprint" }
      if ($SignerSubject -and $auth.SignerCertificate.Subject -ne $SignerSubject) { throw "install: $($setup.name) is signed by '$($auth.SignerCertificate.Subject)', not '$SignerSubject'" }
      Write-Host "signed by $($auth.SignerCertificate.Subject)"
    } elseif ($auth.Status -eq 'Valid') {
      Write-Host "signed by $($auth.SignerCertificate.Subject)"
    }

    $dir = Join-Path $env:LOCALAPPDATA 'Powerrr'
    Get-Process | Where-Object { $_.Path -like "$dir\*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    $run = Start-Process $file -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "install: the installer exited with $($run.ExitCode)" }
    $exe = Join-Path $dir 'powerrr.exe'
    if (-not (Test-Path $exe)) { throw "install: the installer ran, but there's no $exe" }
    Write-Host "installed $exe"
    Start-Process $exe
    Write-Host "it's in the tray (the ^ by the clock if it's hidden), and Ctrl+Alt+P shows or hides it"
  } finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
  }
}
