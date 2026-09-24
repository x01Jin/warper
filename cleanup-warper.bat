@echo off
rem cleanup-warper.bat -- removes Warper portable traces only.
rem Scope, exact:
rem   1. warper.exe process tree
rem   2. orphaned warp-cli.exe --listen listeners spawned by Warper
rem   3. residual warp-cli.exe processes outliving Warper
rem   4. file warper-settings.json beside this script
rem   5. dir warper.exe.WebView2 beside this script
rem   6. dir %%LOCALAPPDATA%%\com.x01jin.warper
rem   7. dir %%APPDATA%%\com.x01jin.warper
rem   8. value HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\Warper
rem   9. key HKCU\SOFTWARE\Classes\AppUserModelId\com.x01jin.warper
rem  10. file %%APPDATA%%\Microsoft\Windows\Start Menu\Programs\Warper.lnk
rem  11. files %%APPDATA%%\Microsoft\Windows\Recent\warper*
rem Cloudflare WARP itself is never touched. No WARP binaries, services,
rem drivers, or accounts are removed. No network, no downloads, no blobs.
rem Pure batch. The only external tools used are built-in commands:
rem taskkill, tasklist, wmic for the --listen sweep, reg, del, rmdir.
setlocal EnableExtensions EnableDelayedExpansion

set "DRY=0"
set "AUTOYES=0"
for %%A in (%*) do (
  if /I "%%~A"=="--dry-run" set "DRY=1"
  if /I "%%~A"=="--yes" set "AUTOYES=1"
)

echo ============================================
echo  Warper portable cleanup
echo ============================================
echo.

if "%DRY%"=="1" goto :dry_run

rem --- 0. Hard admin gate: no relaunch, fail fast -----------------------------
net session >nul 2>&1
if %errorlevel%==0 goto :have_admin
echo [!!] Administrator rights required.
echo Right-click cleanup-warper.bat and choose Run as administrator.
echo Or open an elevated Command Prompt and run cleanup-warper.bat.
exit /b 1
:have_admin

rem --- 1. Confirm unless --yes -------------------------------------------------
if "%AUTOYES%"=="1" goto :kill_phase
echo This will delete all Warper traces:
echo  - %~dp0warper-settings.json -- portable settings file
echo  - %~dp0warper.exe.WebView2 -- portable WebView2 data
echo  - %%LOCALAPPDATA%%\com.x01jin.warper -- WebView2 profile
echo  - %%APPDATA%%\com.x01jin.warper -- settings and logs
echo  - HKCU Run Warper value -- autostart entry
echo  - HKCU AppUserModelId com.x01jin.warper key -- toast registration
echo  - Start Menu Warper.lnk -- toast shortcut
echo  - Recent warper links -- Explorer recent items
echo.
echo Press Y to delete, N to abort.
choice /C YN /N
if %errorlevel%==2 (
  echo Aborted -- nothing deleted.
  exit /b 1
)

:kill_phase
rem --- 2. Kill phase -----------------------------------------------------------
taskkill /F /IM warper.exe /T >nul 2>&1
if %errorlevel%==0 (
  echo [ok] terminated warper.exe process tree
) else (
  echo [..] warper.exe not running
)

set "LISTEN_PIDS="
for /f "skip=1" %%P in ('wmic process where "name='warp-cli.exe' and commandline like '%%%%--listen%%%%'" get processid 2^>nul') do (
  for %%Q in (%%P) do (
    echo %%Q | findstr /R "^[0-9][0-9]*$" >nul
    if !errorlevel!==0 set "LISTEN_PIDS=!LISTEN_PIDS! %%Q"
  )
)
if defined LISTEN_PIDS (
  for %%Q in (!LISTEN_PIDS!) do taskkill /F /PID %%Q >nul 2>&1
  echo [ok] terminated orphaned listeners: !LISTEN_PIDS!
) else (
  echo [..] no orphaned WARP listeners
)

set "RESIDUAL="
for /f "skip=2 tokens=2" %%P in ('tasklist /FI "IMAGENAME eq warp-cli.exe" 2^>nul') do (
  for %%Q in (%%P) do (
    echo %%Q | findstr /R "^[0-9][0-9]*$" >nul
    if !errorlevel!==0 set "RESIDUAL=!RESIDUAL! %%Q"
  )
)
if defined RESIDUAL (
  for %%Q in (!RESIDUAL!) do taskkill /F /PID %%Q >nul 2>&1
  echo [ok] swept residual warp-cli.exe: !RESIDUAL!
) else (
  echo [..] no residual warp-cli.exe
)

rem --- 3. Verify death before any deletion -------------------------------------
timeout /T 2 /NOBREAK >nul
set "STILL=0"
tasklist /FI "IMAGENAME eq warper.exe" 2>nul | find /I "warper.exe" >nul
if %errorlevel%==0 set "STILL=1"
if defined LISTEN_PIDS (
  for %%Q in (!LISTEN_PIDS!) do (
    tasklist /FI "PID eq %%Q" 2>nul | find "%%Q" >nul
    if !errorlevel!==0 set "STILL=1"
  )
)
tasklist /FI "IMAGENAME eq warp-cli.exe" 2>nul | find /I "warp-cli.exe" >nul
if %errorlevel%==0 set "STILL=1"
if "%STILL%"=="1" (
  echo.
  echo [!!] processes survived termination - close them manually and re-run.
  echo No files were deleted.
  pause
  exit /b 1
)
echo [ok] all Warper processes are gone
echo.

rem --- 4. Delete phase, per-item ok/missing ------------------------------------
call :del_retry "%~dp0warper-settings.json" "%~dp0warper-settings.json -- portable settings file"
call :rmdir_retry "%~dp0warper.exe.WebView2" "%~dp0warper.exe.WebView2 -- portable WebView2 data"
call :rmdir_retry "%LOCALAPPDATA%\com.x01jin.warper" "%%LOCALAPPDATA%%\com.x01jin.warper -- WebView2 profile"
call :rmdir_retry "%APPDATA%\com.x01jin.warper" "%%APPDATA%%\com.x01jin.warper -- settings and logs"
reg delete "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" /v Warper /f >nul 2>&1
if %errorlevel%==0 (
  echo [ok] removed HKCU Run Warper value -- autostart entry
) else (
  echo [..] missing HKCU Run Warper value -- autostart entry
)
reg delete "HKCU\SOFTWARE\Classes\AppUserModelId\com.x01jin.warper" /f >nul 2>&1
if %errorlevel%==0 (
  echo [ok] removed HKCU AppUserModelId com.x01jin.warper key -- toast registration
) else (
  echo [..] missing HKCU AppUserModelId com.x01jin.warper key -- toast registration
)
call :del_retry "%APPDATA%\Microsoft\Windows\Start Menu\Programs\Warper.lnk" "Start Menu Warper.lnk -- toast shortcut"
call :del_retry "%APPDATA%\Microsoft\Windows\Recent\warper*" "Recent warper links -- Explorer recent items"

echo.
echo Done. Cloudflare WARP was left untouched.
if "%AUTOYES%"=="0" pause
exit /b 0

:dry_run
echo [dry] check admin with net session -- skipped, dry run touches nothing
echo [dry] kill warper.exe process tree -- taskkill /F /IM warper.exe /T
echo [dry] sweep orphaned warp-cli.exe --listen listeners via wmic
echo [dry] sweep residual warp-cli.exe processes
echo [dry] verify all Warper processes are gone before deletion
echo [dry] delete %~dp0warper-settings.json -- portable settings file
echo [dry] remove %~dp0warper.exe.WebView2 -- portable WebView2 data
echo [dry] remove %%LOCALAPPDATA%%\com.x01jin.warper -- WebView2 profile
echo [dry] remove %%APPDATA%%\com.x01jin.warper -- settings and logs
echo [dry] delete HKCU Run Warper value -- autostart entry
echo [dry] delete HKCU AppUserModelId com.x01jin.warper key -- toast registration
echo [dry] delete Start Menu Warper.lnk -- toast shortcut
echo [dry] delete Recent warper links -- Explorer recent items
echo [dry] done -- nothing deleted
exit /b 0

rem --- Retry removal of a directory whose handles release asynchronously ------
:rmdir_retry
setlocal
set "TARGET=%~1"
set "LABEL=%~2"
if not exist "%TARGET%" (
  echo [..] missing %LABEL%
  exit /b 0
)
for /L %%i in (1,1,6) do (
  rmdir /S /Q "%TARGET%" >nul 2>&1
  if not exist "%TARGET%" (
    echo [ok] removed %LABEL%
    exit /b 0
  )
  timeout /T 1 /NOBREAK >nul
)
echo [!!] kept %LABEL% -- still locked
exit /b 1

rem --- Retry removal of a file pattern whose handles release asynchronously ---
:del_retry
setlocal
set "TARGET=%~1"
set "LABEL=%~2"
if not exist "%TARGET%" (
  echo [..] missing %LABEL%
  exit /b 0
)
for /L %%i in (1,1,6) do (
  del /F /Q "%TARGET%" >nul 2>&1
  if not exist "%TARGET%" (
    echo [ok] removed %LABEL%
    exit /b 0
  )
  timeout /T 1 /NOBREAK >nul
)
echo [!!] kept %LABEL% -- still locked
exit /b 1
