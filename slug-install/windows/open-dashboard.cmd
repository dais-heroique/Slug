@echo off
rem Open the Slug control dashboard as its OWN app window (Edge/Chrome --app),
rem so it looks like a native app rather than a browser tab on localhost.
set "URL=http://127.0.0.1:7333/dashboard"
where msedge >nul 2>&1 && ( start "" msedge --app=%URL% --window-size=1280,860 & goto :eof )
where chrome >nul 2>&1 && ( start "" chrome --app=%URL% --window-size=1280,860 & goto :eof )
start "" "%URL%"
