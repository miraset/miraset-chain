@echo off
setlocal

set ROOT=%~dp0\..\..
set ROOT=%ROOT:\=/%
set ROOT=%ROOT:/=\%
set BUILD=%ROOT%\build

if not exist "%BUILD%" (
  echo Build folder not found: %BUILD%
  echo Run tools\launcher\build-and-package.bat first.
  exit /b 1
)

if not exist "%BUILD%\miraset.exe" (
  echo Missing %BUILD%\miraset.exe
  exit /b 1
)

if not exist "%BUILD%\miraset-worker.exe" (
  echo Missing %BUILD%\miraset-worker.exe
  exit /b 1
)

if not exist "%BUILD%\wallet-miraset.exe" (
  echo Missing %BUILD%\wallet-miraset.exe
  exit /b 1
)

echo Starting MIRASET node...
start "miraset-node" "%BUILD%\miraset.exe" node start

ping 127.0.0.1 -n 3 > nul

echo Starting MIRASET worker...
start "miraset-worker" "%BUILD%\miraset-worker.exe"

ping 127.0.0.1 -n 2 > nul

echo Starting MIRASET wallet GUI...
start "miraset-wallet" "%BUILD%\wallet-miraset.exe"

echo Launcher started. Close windows to stop services.
endlocal

