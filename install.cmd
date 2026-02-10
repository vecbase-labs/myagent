@echo off
curl -fsSL https://raw.githubusercontent.com/vecbase-labs/myagent/main/install.ps1 -o %TEMP%\install_myagent.ps1
powershell -ExecutionPolicy Bypass -File %TEMP%\install_myagent.ps1
del %TEMP%\install_myagent.ps1
