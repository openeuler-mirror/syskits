#!/bin/bash

# Check if git tool exists
if ! command -v git &> /dev/null; then
    echo "git tool not found, unable to get the latest commit ID"
    exit 1
fi

# Get the short version of the latest commit ID
LATEST_COMMIT_ID=$(git rev-parse --short HEAD)

# Check if getting the commit ID was successful
if [ $? -ne 0 ]; then
    echo "Failed to get the latest commit ID"
    exit 1
fi

# Update the Release field in the spec file
SPEC_FILE="syskits.spec"

if [ ! -f "$SPEC_FILE" ]; then
    echo "spec file not found: $SPEC_FILE"
    exit 1
fi

# Use sed to update the Release field in the spec file
sed -i "s/^Release:.*/Release:        dev.$LATEST_COMMIT_ID/" "$SPEC_FILE"

echo "spec file updated: $SPEC_FILE"
