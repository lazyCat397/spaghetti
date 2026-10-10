@echo off
rem Spaghetti: builds spg.exe and installs it (interactive).
rem   build.bat                      asks what you want (recommended)
rem   build.bat -Yes                 accepts every default, asks nothing
rem   build.bat -InstallDir D:\spg   choose the install folder up front
rem   build.bat -NoBuild             install the existing target\release\spg.exe
rem Everything is done by install.ps1; this file only starts it.
setlocal
cd /d "%~dp0"
title Spaghetti installer

where powershell >nul 2>nul
if errorlevel 1 (
  echo [error] PowerShell was not found. It ships with Windows 7 and later.
  goto :end
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
set "RC=%errorlevel%"

:end
if not defined RC set "RC=1"
rem When started by double-click, keep the window open so the result can be read.
echo %cmdcmdline% | find /i "/c" >nul
if not errorlevel 1 (
  echo.
  if "%RC%"=="0" (echo Finished.) else (echo Finished with errors ^(code %RC%^).)
  pause
)
endlocal & exit /b %RC%
