read -p "Chemin vers l'executable > " EXEPATH
xattr -d com.apple.quarantine "$EXEPATH"
chmod +x "$EXEPATH"