TARGET="${1:-aarch64-apple-darwin}"

cargo build --release --target $TARGET
cargo bundle --release --target $TARGET
FOLDER="target/$TARGET/release/bundle/osx"
APP="$FOLDER/Recit.app"

mv "$APP/Contents/MacOS/recit" "$APP/Contents/MacOS/recit-tui"

cat > "$APP/Contents/MacOS/recit" <<'SH'
#!/bin/zsh
APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$APP_DIR/MacOS/recit-tui"

osascript -e 'tell application "Terminal"
    activate
    do script quoted form of POSIX path of "'"$BIN"'"
end tell'
SH

chmod +x "$APP/Contents/MacOS/recit"

codesign --force --deep --sign - $APP
codesign --verify --deep --strict --verbose=2 $APP

ditto -c -k --keepParent $APP "$FOLDER/Recit-macos.zip"