@echo off
rem Compila (se o codigo mudou) e inicia o CeresMC Server a partir desta pasta.
rem server.properties, ops.txt, data\ e worlds\ ficam aqui.
cd /d "%~dp0"
title CeresMC Server
echo Compilando (leva poucos segundos se nada mudou)...
cargo build --release
if errorlevel 1 (
    echo.
    echo Falha ao compilar. Se aparecer "failed to remove file", o servidor ja esta aberto em outra janela.
    pause
    exit /b 1
)
echo.
echo Para desligar salvando tudo: digite stop ou aperte Ctrl+C
echo.
"target\release\ceres.exe"
echo.
echo Servidor desligado.
pause
