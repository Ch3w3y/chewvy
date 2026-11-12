#!/usr/bin/env bash
# Print the Debian source package and version behind every library in an
# AppDir's usr/lib. The list is published beside the AppImage so anyone can
# fetch the exact source each bundled library was built from. Run it on the
# Debian system that assembled the AppDir, before its packages change.
set -euo pipefail

appdir=${1:?usage: bundled-sources.sh APPDIR}
command -v dpkg-query >/dev/null || {
    echo "bundled-sources.sh: needs dpkg-query (run on the Debian build system)" >&2
    exit 1
}

echo "# Libraries bundled in the Chewvy AppImage and the Debian source package of each."
echo "# Source for any row: https://snapshot.debian.org/package/<source package>/<version>/"
printf '# library\tbinary package\tsource package\tversion\n'
for lib in "$appdir"/usr/lib/*; do
    name=$(basename "$lib")
    owner=$(dpkg-query -S "*/$name" 2>/dev/null | head -n 1) || owner=""
    package=${owner%%: *}
    if [[ -z "$package" ]]; then
        echo "bundled-sources.sh: $name does not belong to a Debian package" >&2
        exit 1
    fi
    printf '%s\t%s\t' "$name" "$package"
    dpkg-query -W -f '${source:Package}\t${source:Version}\n' "$package"
done
