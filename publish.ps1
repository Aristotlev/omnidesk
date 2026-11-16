# Publishes a new OmniDesk version: GitHub builds it, every installed copy updates itself,
# and the new installer lands on your Desktop.
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot
$errFile = Join-Path $PSScriptRoot 'build-error.txt'
$logFile = Join-Path $PSScriptRoot 'build-log.txt'
Remove-Item $errFile, $logFile -ErrorAction SilentlyContinue
function Log($m) { Write-Host $m -ForegroundColor Cyan; Add-Content $logFile $m }
function Fail($m) { Log "FAILED: $m"; Add-Content $errFile $m; exit 1 }

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Fail 'GitHub CLI (gh) not found' }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Fail 'git not found' }

gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
  Log 'Sign in to GitHub in the browser window that opens...'
  gh auth login --hostname github.com --web --git-protocol https
  if ($LASTEXITCODE -ne 0) { Fail 'GitHub sign-in failed' }
}
$scopes = (gh auth status 2>&1 | Out-String)
if ($scopes -notmatch 'workflow') {
  Log 'Approve the extra "workflow" permission in the browser...'
  gh auth refresh --hostname github.com --scopes workflow
}
gh auth setup-git *> $null
$login = gh api user --jq .login
$id = gh api user --jq .id
if (-not $login) { Fail 'Could not read GitHub user' }
$repo = "$login/omnidesk"

# Workflows: GitHub reads .github\workflows. Source copies live in ci\.
$wf = Join-Path $PSScriptRoot '.github\workflows'
New-Item -ItemType Directory -Force $wf | Out-Null
Get-ChildItem $wf -Filter *.yml | Remove-Item -Force
Copy-Item (Join-Path $PSScriptRoot 'ci\*.yml') $wf -Force

if (-not (Test-Path .git)) { git init -b main | Out-Null }
# Never let the signing keys reach git.
git check-ignore -q 'release-keys/updater.key'
if ($LASTEXITCODE -ne 0) { Fail 'release-keys is not in .gitignore' }
if (-not (git config user.email)) {
  git config user.email "$id+$login@users.noreply.github.com"
  git config user.name $login
}
gh repo view $repo *> $null
if ($LASTEXITCODE -ne 0) {
  gh repo create omnidesk --public --source . --remote origin
  if ($LASTEXITCODE -ne 0) { Fail 'Could not create GitHub repo' }
}
if (-not (git remote)) { git remote add origin "https://github.com/$repo.git" }

# Releases must be public so installed apps can download updates.
$vis = gh repo view $repo --json visibility --jq .visibility
if ($vis -ne 'PUBLIC') {
  Log 'Making the repo public so updates can be downloaded...'
  gh repo edit $repo --visibility public --accept-visibility-change-consequences
  if ($LASTEXITCODE -ne 0) { gh api -X PATCH "repos/$repo" -f visibility=public | Out-Null }
}

# What people see on the GitHub page.
gh repo edit $repo --description 'Every screen you own on one desk: see all your computers live, control any of them, share copied text. Windows, Mac and Linux.' --homepage 'https://aristotlev.github.io/omnidesk/' --add-topic remote-desktop --add-topic kvm --add-topic screen-sharing --add-topic multi-device --add-topic windows --add-topic macos --add-topic linux *> $null

# Signing key for updates (stays on this PC and in GitHub's encrypted secrets).
$keyFile = Join-Path $PSScriptRoot 'release-keys\updater.key'
$pwFile = Join-Path $PSScriptRoot 'release-keys\updater.password'
if (-not (Test-Path $keyFile)) { Fail 'release-keys\updater.key is missing' }
gh secret set TAURI_SIGNING_PRIVATE_KEY -R $repo --body (Get-Content $keyFile -Raw).Trim()
if ($LASTEXITCODE -ne 0) { Fail 'Could not save the signing key to GitHub' }
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -R $repo --body (Get-Content $pwFile -Raw).Trim()

Log 'Update signing key saved on GitHub.'

# Version: bump the patch number if this version was already released.
$pkgPath = Join-Path $PSScriptRoot 'package.json'
$pkg = Get-Content $pkgPath -Raw
$version = ([regex]::Match($pkg, '"version":\s*"([^"]+)"')).Groups[1].Value
git fetch --tags -q origin 2>$null
if (git tag -l "v$version") {
  $p = $version.Split('.')
  $version = "$($p[0]).$($p[1]).$([int]$p[2] + 1)"
  $pkg = [regex]::Replace($pkg, '"version":\s*"[^"]+"', "`"version`": `"$version`"", 1)
  [IO.File]::WriteAllText($pkgPath, $pkg)
}
Log "Publishing OmniDesk v$version"

git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
  git commit -q -m "OmniDesk v$version" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`nClaude-Session: https://claude.ai/code/session_019ndGfmtgAuG7kjkB975Yvj"
}
git push -u origin main
if ($LASTEXITCODE -ne 0) { Fail 'git push failed' }
git tag "v$version"
git push origin "v$version"
if ($LASTEXITCODE -ne 0) { Fail 'Could not push the version tag' }

# Website (home, privacy, terms) served by GitHub Pages from docs\.
gh api -X POST "repos/$repo/pages" -f 'source[branch]=main' -f 'source[path]=/docs' *> $null

Log 'Waiting for GitHub to start the build...'
$runId = $null
for ($i = 0; $i -lt 30 -and -not $runId; $i++) {
  Start-Sleep 5
  $runs = gh run list -R $repo --workflow release.yml --limit 10 --json databaseId,headBranch | ConvertFrom-Json
  $runId = ($runs | Where-Object { $_.headBranch -eq "v$version" } | Select-Object -First 1).databaseId
}
if (-not $runId) { Fail 'Build did not start on GitHub' }
Log "Building v$version on GitHub (run $runId). About 15 minutes..."
gh run watch $runId -R $repo --exit-status --interval 20
if ($LASTEXITCODE -ne 0) {
  gh run view $runId -R $repo --log-failed | Out-File $errFile -Encoding utf8
  Log 'Part of the build failed. Details in build-error.txt'
}

$dest = Join-Path ([Environment]::GetFolderPath('Desktop')) 'OmniDesk'
New-Item -ItemType Directory -Force $dest | Out-Null
gh release download "v$version" -R $repo -p '*-setup.exe' -D $dest --clobber
$setup = Get-ChildItem $dest -Filter "*$version*-setup.exe" | Select-Object -First 1
if (-not $setup) { Fail 'Windows installer not found in the release. Details in build-error.txt' }
$assets = (gh release view "v$version" -R $repo --json assets --jq '.assets[].name' | Out-String)
foreach ($n in 'OmniDesk-Windows.exe', 'OmniDesk-Mac.dmg', 'OmniDesk-Linux.AppImage') {
  if ($assets -match [regex]::Escape($n)) { Log "Download link: https://github.com/$repo/releases/latest/download/$n" }
  else { Log "$n is missing from this release (see build-error.txt)" }
}
Log "Released v$version. Download page for users: https://github.com/$repo/releases/latest"

# Update the copy installed on this PC: close it, install silently, reopen it.
Log 'Installing on this PC...'
Get-Process -Name 'OmniDesk', 'omnidesk' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2
$p = Start-Process $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "Installer exited with code $($p.ExitCode)" }
$exe = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'OmniDesk') -Filter '*.exe' -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -notmatch 'uninstall' } | Select-Object -First 1
if ($exe) { Start-Process $exe.FullName }
Log "DONE: v$version installed and running. Installer copy: $($setup.FullName)"
