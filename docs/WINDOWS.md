# Windows support

The Rust backend and Vue dashboard support Windows 10/11 with a native Windows process collector. No WSL, Python subprocess, administrator account, Windows service, or per-scan PowerShell invocation is required.

## Build and run

Install Rust's MSVC toolchain, Visual Studio C++ Build Tools with a Windows SDK, Node 20+, and pnpm 9+. In PowerShell from the repository root:

```powershell
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
cargo test --all-targets --manifest-path backend/Cargo.toml
cargo run --release --manifest-path backend/Cargo.toml -- --web-dir frontend/dist
```

Open `http://127.0.0.1:8765`. The default data directory is `%LOCALAPPDATA%\headless-watch\data`. `--data-dir` and `--web-dir` accept paths containing spaces. Development instances run in the foreground and exit with Ctrl+C; `start` and `stop` manage an installed instance.

## Installation and login startup

```powershell
.\install.ps1 -DryRun
.\install.ps1
& "$env:LOCALAPPDATA\headless-watch\bin\headless-watch.exe" status
& "$env:LOCALAPPDATA\headless-watch\bin\headless-watch.exe" stop
& "$env:LOCALAPPDATA\headless-watch\bin\headless-watch.exe" start
& "$env:LOCALAPPDATA\headless-watch\bin\headless-watch.exe" open
```

If PowerShell's local script policy prevents running the installer, invoke the file for this process with `powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1`. This does not change the machine's execution policy. Organization policy can still prevent scripts or Startup entries.

Installation builds from source, or copies prebuilt `bin/` and `web/` files from an extracted Windows release. Files live under `%LOCALAPPDATA%\headless-watch`:

| Path | Purpose |
|---|---|
| `bin/headless-watch.exe` | Native backend and lifecycle commands |
| `web/` | Built dashboard |
| `data/` | SQLite history, settings, active-monitor metadata |
| `logs/` | Background stdout/stderr |
| `install.json` | Installation marker, port and absolute data/web paths |
| `launch.ps1` | Current-user login launcher |

A shortcut named **Headless Watch** is added to the current user's Windows Startup folder. Its hidden PowerShell launcher calls the installed executable's `start` command; the monitor itself uses a native hidden process. No PowerShell process remains resident for collection. Startup occurs after interactive login and ends with that user session. There is no automatic crash restart.

Options:

- `-InstallDir PATH` selects a dedicated application directory.
- `-Port 9100` changes the installed port; use `status --port 9100` and `open --port 9100` for that port.
- `-NoAutoStart` avoids changing the Startup shortcut.
- `-SkipStart` installs without starting the monitor immediately.
- `-SkipBuild` uses existing release/dashboard artifacts in a source checkout.
- `-DryRun` prints the plan without building, copying, stopping processes, or changing startup.

The installer stops an existing verified installed monitor before copying its executable. Existing nonempty installation directories require a matching installation marker. If a source run has already created data at the default location, use a separate `-InstallDir` and keep the source data directory. Only one login shortcut can own the standard name; another installation must use `-NoAutoStart` or remove the first installation. This installer does not add a directory to PATH.

`start` keeps one instance per data directory, starts without a console window, waits for local HTTP health, and redirects output to `logs/headless-watch.log`. Logs larger than 2 MiB rotate once on the next start. Log growth during a long continuous run is not capped. `stop` verifies the PID, precise process creation time, and canonical executable path before signaling a random native shutdown event. It never runs `taskkill /IM` or kills a process by executable name. Browser processes remain running when the monitor stops.

## Uninstall and data retention

```powershell
& "$env:LOCALAPPDATA\headless-watch\uninstall.ps1" -DryRun
& "$env:LOCALAPPDATA\headless-watch\uninstall.ps1"             # keep SQLite data and logs
& "$env:LOCALAPPDATA\headless-watch\uninstall.ps1" -RemoveData # explicitly remove them
```

For a custom installation pass the same `-InstallDir`. The uninstaller requires the matching installation marker, validates removal targets, refuses junctions/symbolic links, removes only its own Startup shortcut, and stops only the identity-verified monitor. Data and logs are preserved by default. Source-run instances or databases outside the installation directory are not removed.

## Native metrics and limits

- Toolhelp snapshots identify process ancestry. CPU uses `GetProcessTimes` kernel plus user time in 100 ns units; memory uses `GetProcessMemoryInfo` working set. The memory sum can count shared pages more than once.
- Creation FILETIME is part of process identity and prevents PID reuse from matching a stale session. The boot identifier uses native boot time; if unavailable, the collector still requires the precise process creation time.
- Current-user SID checks constrain collection and termination. The API's legacy `uid` field is a compatibility marker on Windows, not a Unix numeric UID.
- UTF-16 process command lines are read natively. Processes that deny access or whose command line cannot be read do not gain headless-browser status from their executable name alone. Elevated or protected processes can be inaccessible.
- Windows has no equivalent supported PSS implementation here. PSS is unavailable rather than substituted with private bytes or working set.
- An ordinary browser Stop cannot safely emulate Linux SIGTERM for every detached process. A graceful API request returns `graceful_unavailable` without signaling. The separately confirmed **Force stop** uses `TerminateProcess` only after checking current process identity, same-user SID, and browser ancestry. It can discard browser work.
- Desktop toast notifications are not implemented. Notification availability is false; dashboard alerts, history, and acknowledgement still work.
- Data files inherit Windows ACLs. The default location is the current user's LocalAppData; custom shared directories do not receive a new restrictive ACL automatically.
- Existing local TCP CDP endpoints work. Remote-debugging pipe sessions remain observable but cannot provide screenshots.
- Linux acceptance/benchmark scripts and systemd commands retain their Linux meaning and are not the Windows test runner. macOS remains unsupported.

## Packaging and validation

```powershell
powershell -File tools/package-release.ps1
```

This writes a Windows ZIP and SHA-256 file under `dist/`. The archive's installer uses prebuilt files without Cargo/pnpm. CI runs Cargo tests, formatting, and the frontend build on both Ubuntu and Windows. Windows lifecycle unit tests cover data-directory singleton locking, shutdown events, and stale creation-time rejection. Installer parser checks and `-DryRun` avoid modifying a CI user's login startup, then a temporary `-NoAutoStart` installation exercises `start` → `status` → `stop` → `uninstall`.

For isolated local verification, install into a disposable directory using `-NoAutoStart`, use a free port, exercise `start/status/stop`, and uninstall that directory. Testing the login trigger itself requires a real interactive sign-out/sign-in and is separate from a build or HTTP health check.

`python tools/windows_smoke.py` exercises a private native backend and Chrome instance, CPU/working-set samples, CDP screenshots, stale-identity rejection, force-stop cleanup, and history across a restart. Python 3.10+ is only needed for this optional test, not for the application. Windows unit/API tests, Chrome acceptance, and temporary-directory installation/lifecycle checks passed locally. Real sign-out/sign-in and the Linux CI run have not been performed in this Windows workspace.
