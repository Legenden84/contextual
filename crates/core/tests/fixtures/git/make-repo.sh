#!/bin/sh
# Builds the scripted repository used by the git parity fixtures:
#   $1/origin.git  bare "GitHub" remote
#   $1/work        the project, on feature/login with staged, unstaged and untracked changes
# Dates and identities are fixed so commits are reproducible. Run with GIT_CONFIG_GLOBAL=/dev/null
# and GIT_CONFIG_NOSYSTEM=1 so the user's git config can't change the output.
set -eu

dir=$1
export GIT_AUTHOR_NAME="Ada Lovelace" GIT_AUTHOR_EMAIL="ada@example.com"
export GIT_COMMITTER_NAME="Ada Lovelace" GIT_COMMITTER_EMAIL="ada@example.com"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

n=0
commit() {
  n=$((n + 1))
  GIT_AUTHOR_DATE="2024-01-0${n}T10:00:00Z" GIT_COMMITTER_DATE="2024-01-0${n}T10:00:00Z" git commit -q -m "$1"
}

git init -q --bare -b main "$dir/origin.git"
git init -q -b main "$dir/work"
cd "$dir/work"
git config user.name "Ada Lovelace"
git config user.email "ada@example.com"
git config commit.gpgsign false
git config core.autocrlf false

printf '# Demo\n\nA project.\n' > README.md
mkdir -p src
printf 'export const a = 1\nexport const b = 2\n' > src/app.ts
printf 'line one\r\nline two\r\n' > crlf.txt
printf 'bin\000ary\001data' > bin.dat
printf 'to be renamed\nkeep this text\nand this\n' > old-name.txt
git add -A
commit "Initial commit"

git remote add origin "$dir/origin.git"
git push -q -u origin main
git remote set-head origin main

# feature/login: two commits, the first one pushed.
git switch -q -c feature/login
printf 'export const a = 1\nexport const b = 3\nexport const login = true\n' > src/app.ts
git mv old-name.txt new-name.txt
printf 'with space\n' > 'file with space.txt'
git add -A
commit "Add login flag"
git push -q -u origin feature/login
git rm -q bin.dat
printf 'line one\r\nline 2\r\n' > crlf.txt
git add -A
commit "Drop binary, edit CRLF file"

# feature/child: forked from feature/login, parent recorded like Contextual does.
git switch -q -c feature/child
printf 'child\n' > child.txt
git add -A
commit "Child work"
git config branch.feature/child.contextualParent feature/login

# main moves on after the fork.
git switch -q main
printf '# Demo\n\nA project on main.\n' > MAIN.md
git add -A
commit "Main moves on"
git push -q origin main

# Working tree on feature/login.
git switch -q feature/login
printf '# Demo\n\nA project, edited.\n' > README.md
printf 'staged\n' > staged-new.txt
git add README.md staged-new.txt
git mv new-name.txt renamed-again.txt
printf 'export const a = 1\nexport const b = 3\nexport const login = false\n' > src/app.ts
rm crlf.txt
mkdir -p notes
printf 'todo\n' > notes/todo.md
printf 'z\n' > Zeta.txt
printf 'a\n' > alpha.txt
