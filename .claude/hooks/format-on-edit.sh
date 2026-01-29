#!/bin/bash
# Auto-format files after editing based on file type

# Extract file path from the JSON input
FILE_PATH=$(jq -r '.tool_input.file_path')

# Skip if no file path
if [ -z "$FILE_PATH" ] || [ "$FILE_PATH" = "null" ]; then
  exit 0
fi

# Format based on file extension
case "$FILE_PATH" in
  *.rs)
    # Rust files - format with rustfmt (edition 2024)
    rustfmt --edition 2024 "$FILE_PATH" 2>/dev/null
    ;;
  *)
    # Unknown file type - no-op
    :
    ;;
esac
