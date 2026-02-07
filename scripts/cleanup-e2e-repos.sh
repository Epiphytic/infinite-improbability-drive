#!/bin/bash
# Cleanup script for e2e test repositories
# Usage: ./scripts/cleanup-e2e-repos.sh [--dry-run] [--all] [--older-than DURATION]

set -euo pipefail

ORG="epiphytic"
PREFIX="e2e-"
DRY_RUN=false
DELETE_ALL=false
OLDER_THAN=""

usage() {
	cat <<EOF
Usage: $0 [OPTIONS]

Delete e2e test repositories from the $ORG organization.

Options:
    --dry-run           List repos that would be deleted without deleting them
    --all               Delete all e2e repos (default: interactive selection)
    --older-than DURATION  Only delete repos older than DURATION
                            Supports: 2h (hours), 7d (days), or just 7 (days)
    -h, --help             Show this help message

Examples:
    $0                      # Interactive: select repos to delete
    $0 --dry-run            # List all e2e repos without deleting
    $0 --all                # Delete all e2e repos (with confirmation)
    $0 --older-than 2h      # Delete repos older than 2 hours
    $0 --older-than 7d      # Delete repos older than 7 days
    $0 --older-than 7       # Delete repos older than 7 days (default unit)
EOF
	exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
	case $1 in
	--dry-run)
		DRY_RUN=true
		shift
		;;
	--all)
		DELETE_ALL=true
		shift
		;;
	--older-than)
		OLDER_THAN="$2"
		shift 2
		;;
	-h | --help)
		usage
		;;
	*)
		echo "Unknown option: $1"
		usage
		;;
	esac
done

# Check gh is available
if ! command -v gh &>/dev/null; then
	echo "Error: gh CLI is not installed or not in PATH"
	exit 1
fi

# Check gh is authenticated
if ! gh auth status &>/dev/null; then
	echo "Error: gh CLI is not authenticated. Run 'gh auth login' first."
	exit 1
fi

echo "Fetching e2e test repositories from $ORG..."

# Get repos with creation date
REPOS_JSON=$(gh repo list "$ORG" --limit 100 --json name,createdAt | jq -r ".[] | select(.name | startswith(\"$PREFIX\"))")

if [[ -z "$REPOS_JSON" ]]; then
	echo "No e2e test repositories found."
	exit 0
fi

# Filter by age if specified
if [[ -n "$OLDER_THAN" ]]; then
	# Parse duration: 2h = 2 hours, 7d = 7 days, 7 = 7 days (default)
	if [[ "$OLDER_THAN" =~ ^([0-9]+)h$ ]]; then
		AMOUNT="${BASH_REMATCH[1]}"
		CUTOFF_DATE=$(date -v-${AMOUNT}H +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -d "$AMOUNT hours ago" --iso-8601=seconds 2>/dev/null)
	elif [[ "$OLDER_THAN" =~ ^([0-9]+)d?$ ]]; then
		AMOUNT="${BASH_REMATCH[1]}"
		CUTOFF_DATE=$(date -v-${AMOUNT}d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -d "$AMOUNT days ago" --iso-8601=seconds 2>/dev/null)
	else
		echo "Error: Invalid duration '$OLDER_THAN'. Use e.g. 2h (hours), 7d (days), or 7 (days)."
		exit 1
	fi
	REPOS=$(echo "$REPOS_JSON" | jq -r "select(.createdAt < \"$CUTOFF_DATE\") | .name")
else
	REPOS=$(echo "$REPOS_JSON" | jq -r '.name')
fi

if [[ -z "$REPOS" ]]; then
	echo "No repositories match the criteria."
	exit 0
fi

REPO_COUNT=$(echo "$REPOS" | wc -l | tr -d ' ')
echo "Found $REPO_COUNT e2e test repository(ies):"
echo ""

# Display repos with details
echo "$REPOS_JSON" | jq -r '[.name, .createdAt] | @tsv' | while IFS=$'\t' read -r name created; do
	if echo "$REPOS" | grep -q "^${name}$"; then
		# Format the date nicely
		created_fmt=$(echo "$created" | cut -d'T' -f1)
		echo "  - $name (created: $created_fmt)"
	fi
done

echo ""

if $DRY_RUN; then
	echo "[DRY RUN] Would delete $REPO_COUNT repository(ies)"
	exit 0
fi

if $DELETE_ALL; then
	echo "⚠️  You are about to delete $REPO_COUNT repository(ies)."
	read -p "Type 'DELETE' to confirm: " confirm
	if [[ "$confirm" != "DELETE" ]]; then
		echo "Aborted."
		exit 1
	fi

	echo ""
	echo "$REPOS" | while read -r repo; do
		echo "Deleting $ORG/$repo..."
		if gh repo delete "$ORG/$repo" --yes; then
			echo "  ✓ Deleted"
		else
			echo "  ✗ Failed to delete"
		fi
	done
else
	# Interactive mode
	echo "Select repositories to delete (space to toggle, enter to confirm):"
	echo ""

	# Use fzf if available, otherwise fall back to simple selection
	if command -v fzf &>/dev/null; then
		SELECTED=$(echo "$REPOS" | fzf --multi --header="Select repos to delete (TAB to select, ENTER to confirm)")

		if [[ -z "$SELECTED" ]]; then
			echo "No repositories selected."
			exit 0
		fi

		SELECTED_COUNT=$(echo "$SELECTED" | wc -l | tr -d ' ')
		echo ""
		echo "Will delete $SELECTED_COUNT repository(ies):"
		echo "$SELECTED" | sed 's/^/  - /'
		echo ""
		read -p "Confirm deletion? [y/N] " confirm

		if [[ "$confirm" =~ ^[Yy]$ ]]; then
			echo "$SELECTED" | while read -r repo; do
				echo "Deleting $ORG/$repo..."
				if gh repo delete "$ORG/$repo" --yes; then
					echo "  ✓ Deleted"
				else
					echo "  ✗ Failed to delete"
				fi
			done
		else
			echo "Aborted."
		fi
	else
		# Simple numbered selection without fzf
		echo "Enter repo numbers to delete (comma-separated), or 'all', or 'q' to quit:"
		echo ""

		i=1
		echo "$REPOS" | while read -r repo; do
			echo "  $i) $repo"
			i=$((i + 1))
		done

		echo ""
		read -p "Selection: " selection

		if [[ "$selection" == "q" ]]; then
			echo "Aborted."
			exit 0
		fi

		if [[ "$selection" == "all" ]]; then
			SELECTED="$REPOS"
		else
			SELECTED=""
			IFS=',' read -ra NUMS <<<"$selection"
			for num in "${NUMS[@]}"; do
				num=$(echo "$num" | tr -d ' ')
				repo=$(echo "$REPOS" | sed -n "${num}p")
				if [[ -n "$repo" ]]; then
					SELECTED="${SELECTED}${repo}"$'\n'
				fi
			done
			SELECTED=$(echo "$SELECTED" | sed '/^$/d')
		fi

		if [[ -z "$SELECTED" ]]; then
			echo "No valid repositories selected."
			exit 0
		fi

		SELECTED_COUNT=$(echo "$SELECTED" | wc -l | tr -d ' ')
		echo ""
		echo "Will delete $SELECTED_COUNT repository(ies):"
		echo "$SELECTED" | sed 's/^/  - /'
		echo ""
		read -p "Confirm deletion? [y/N] " confirm

		if [[ "$confirm" =~ ^[Yy]$ ]]; then
			echo "$SELECTED" | while read -r repo; do
				[[ -z "$repo" ]] && continue
				echo "Deleting $ORG/$repo..."
				if gh repo delete "$ORG/$repo" --yes; then
					echo "  ✓ Deleted"
				else
					echo "  ✗ Failed to delete"
				fi
			done
		else
			echo "Aborted."
		fi
	fi
fi

echo ""
echo "Done."
