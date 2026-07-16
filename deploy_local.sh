#!/usr/bin/env bash
set -e

# Resolve paths relative to the script's location securely
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
NASFILES_DIR="${SCRIPT_DIR}"
INFRA_DIR="${SCRIPT_DIR}/../_infra/home-stacks/nasfiles"
SSH_KEY_DIR="${SCRIPT_DIR}/../_infra/.ssh"

# Ensure the infra directory exists before attempting state queries
if [ ! -d "$INFRA_DIR" ]; then
    echo "❌ Error: Infra directory not found at $INFRA_DIR"
    exit 1
fi

echo "➡️ Fetching internal configuration dynamically from OpenTofu..."
cd "$INFRA_DIR"
TRUENAS_IP=$(echo "local.truenas_ip" | tofu console | tr -d '"' | tr -d '\n')
DOMAIN_NAME=$(echo "local.domain_name" | tofu console | tr -d '"' | tr -d '\n')
REGISTRY_USER=$(echo "data.terraform_remote_state.core_k8s.outputs.registry_pull_username" | tofu console | tr -d '"' | tr -d '\n')
REGISTRY_PASS=$(echo "data.terraform_remote_state.core_k8s.outputs.registry_pull_password" | tofu console | tr -d '"' | tr -d '\n')
IMAGE="registry.${DOMAIN_NAME}/opensource/nasfiles"

if [ -z "$TRUENAS_IP" ] || [ -z "$DOMAIN_NAME" ] || [ -z "$REGISTRY_USER" ]; then
    echo "❌ Error: Could not retrieve configuration from OpenTofu."
    exit 1
fi

# Locate the appropriate SSH key for the TrueNAS remote
SSH_KEY="${SSH_KEY_DIR}/truenas_ed25519"
if [ ! -f "$SSH_KEY" ]; then
    echo "❌ Error: SSH key not found at $SSH_KEY"
    exit 1
fi

echo "➡️ Syncing local nasfiles repository with origin/main..."
cd "$NASFILES_DIR"
git pull origin main

echo "➡️ Scanning registry for the newest published image tag..."
COMMIT=""
for hash in $(git log -n 20 --format="%H"); do
    STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u "$REGISTRY_USER:$REGISTRY_PASS" "https://registry.${DOMAIN_NAME}/v2/opensource/nasfiles/manifests/$hash")
    if [ "$STATUS" = "200" ]; then
        COMMIT="$hash"
        break
    fi
done

if [ -z "$COMMIT" ]; then
    echo "❌ Error: None of the 20 most recent commits are published in the registry."
    exit 1
fi
echo "✅ Newest registry-published commit resolved: $COMMIT"

cd "$INFRA_DIR"
echo "➡️ Updating infrastructure configuration (nasfiles.tf)..."
# Use sed to aggressively rewrite any existing image tag to the latest commit. 
if [[ "$OSTYPE" == "darwin"* ]]; then
  sed -i '' -E "s|image[[:space:]]*=[[:space:]]*\"${IMAGE}:[a-zA-Z0-9_-]+\"|image   = \"${IMAGE}:${COMMIT}\"|" nasfiles.tf
else
  sed -i -E "s|image[[:space:]]*=[[:space:]]*\"${IMAGE}:[a-zA-Z0-9_-]+\"|image   = \"${IMAGE}:${COMMIT}\"|" nasfiles.tf
fi
echo "✅ Configuration updated safely."

echo "➡️ Pre-pulling new docker image dynamically on TrueNAS to avert auth race conditions..."
ssh -o StrictHostKeyChecking=no -i "$SSH_KEY" "root@${TRUENAS_IP}" "docker pull ${IMAGE}:${COMMIT}"
echo "✅ Image cached locally."

echo "➡️ Deploying application configuration via OpenTofu..."
tofu apply -auto-approve

echo "✨ NASDrive local stack update complete!"
