@echo off
setlocal
cd /d "%~dp0.."
if "%1"=="dev" goto DEV
if "%1"=="build" goto BUILD
goto RUN

:DEV
start "exam-server" cmd /c cargo run -p exam-server
cd web
call npm install
call npm run dev
goto END

:BUILD
cd web
call npm install
call npm run build
cd ..
goto END

:RUN
if exist "web\dist\teacher.html" goto SERVE
echo Missing web\dist, building frontend first.
cd web
call npm install
call npm run build
cd ..

:SERVE
set EXAM_WEB_DIR=%CD%\web\dist
cargo run -p exam-server
goto END

:END
endlocal
