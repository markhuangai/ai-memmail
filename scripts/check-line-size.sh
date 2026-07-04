#!/usr/bin/env bash
set -euo pipefail

max_lines="${MAX_FILE_LINES:-700}"
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

violations=()
while IFS= read -r -d '' file; do
  [[ -f "$file" ]] || continue
  case "$file" in
    Cargo.lock|web/package-lock.json|web/dist/*|web/node_modules/*|target/*)
      continue
      ;;
  esac
  case "$file" in
    *.rs|*.ts|*.tsx|*.js|*.jsx|*.mjs|*.cjs|*.css|*.sh|*.toml|*.yaml|*.yml|*.md|*.sql)
      ;;
    *)
      continue
      ;;
  esac

  line_count="$(wc -l < "$file" | tr -d '[:space:]')"
  if (( line_count > max_lines )); then
    violations+=("${line_count} ${file}")
  fi
done < <(git ls-files -z --cached --others --exclude-standard)

if ((${#violations[@]} > 0)); then
  printf 'Files exceed %s lines:\\n' "$max_lines" >&2
  printf '%s\\n' "${violations[@]}" | sort -rn >&2
  exit 1
fi
