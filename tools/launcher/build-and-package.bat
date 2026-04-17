@echo off
setlocal

set ROOT=%~dp0\..\..
set ROOT=%ROOT:\=/%
set ROOT=%ROOT:/=\%
set BUILD=%ROOT%\build
set RELEASES=%ROOT%\releases

if not exist "%BUILD%" mkdir "%BUILD%"
if not exist "%RELEASES%" mkdir "%RELEASES%"

echo Building node + worker + launcher (release)...
cargo build --release --bin miraset --bin miraset-worker --bin miraset-launcher
if errorlevel 1 exit /b 1

echo Building wallet (tauri)...
pushd "%ROOT%\wallet"
bunx tauri build
if errorlevel 1 (
  popd
  exit /b 1
)
popd

echo Copying artifacts...
copy /y "%ROOT%\target\release\miraset.exe" "%BUILD%\miraset.exe" > nul
copy /y "%ROOT%\target\release\miraset-worker.exe" "%BUILD%\miraset-worker.exe" > nul
copy /y "%ROOT%\target\release\miraset-launcher.exe" "%BUILD%\miraset-launcher.exe" > nul
copy /y "%ROOT%\wallet\src-tauri\target\release\wallet-miraset.exe" "%BUILD%\wallet-miraset.exe" > nul

if not exist "%BUILD%\miraset.exe" (
  echo Missing %BUILD%\miraset.exe
  exit /b 1
)
if not exist "%BUILD%\miraset-worker.exe" (
  echo Missing %BUILD%\miraset-worker.exe
  exit /b 1
)
if not exist "%BUILD%\miraset-launcher.exe" (
  echo Missing %BUILD%\miraset-launcher.exe
  exit /b 1
)
if not exist "%BUILD%\wallet-miraset.exe" (
  echo Missing %BUILD%\wallet-miraset.exe
  exit /b 1
)

echo Creating release archive...
powershell -NoProfile -Command "Compress-Archive -Path '%BUILD%\*' -DestinationPath '%RELEASES%\miraset-windows.zip' -Force"

if errorlevel 1 exit /b 1

echo Done: %RELEASES%\miraset-windows.zip
endlocal

