#!/usr/bin/env bash
# Print the Homebrew formula and version behind every library in the app
# bundle's Frameworks folder. The list is published beside the disk image so
# anyone can fetch the source each bundled library was built from. Run it on
# the machine that built the bundle, before its Homebrew packages change.
set -euo pipefail

app=${1:?usage: bundled-sources.sh Chewvy.app}
cellar=$(brew --cellar)

echo "# Libraries bundled in Chewvy.app and the Homebrew formula of each."
echo "# Source for any row: brew fetch --build-from-source <formula>, or the"
echo "# formula file at https://github.com/Homebrew/homebrew-core (Formula/)."
printf '# library\tformula\tversion\n'
for lib in "$app"/Contents/Frameworks/*.dylib; do
    name=$(basename "$lib")
    origin=$(find "$cellar" -path "*/lib/$name" -print -quit)
    if [[ -z $origin ]]; then
        echo "bundled-sources.sh: $name does not come from a Homebrew formula" >&2
        exit 1
    fi
    relative=${origin#"$cellar"/}
    formula=${relative%%/*}
    rest=${relative#*/}
    printf '%s\t%s\t%s\n' "$name" "$formula" "${rest%%/*}"
done
