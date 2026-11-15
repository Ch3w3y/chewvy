# Publish the website

The website is the static files in `site/`. Nothing builds it. It was first
published with 0.4.0 on 2026-11-15.

Publish the site when the release it describes is published, not before. The
page links to the newest release's packages, so a visitor must be able to
download what the page shows.

## Before you publish

1. Publish the GitHub release.
2. Rerun the captures with the build that was released, so that the pictures
   match the app:

   ```sh
   site/tools/capture.sh
   ```

3. In `site/index.html`, change `data-release="pending"` on the `<html>` tag
   to `data-release="shipped"`. The "In the next release" label on the
   Backdrops section then disappears. The workflow refuses to publish while
   the value is `pending`. To publish with the label showing, remove that
   step from the workflow.
4. Check the page:

   ```sh
   python3 site/tools/check-links.py --external
   python3 -m http.server 8765 --bind 127.0.0.1 --directory site
   ```

5. Commit the changes to `main`.

## Publish with GitHub Actions (recommended)

The workflow is `.github/workflows/pages.yml`, and Pages is set to build from
GitHub Actions (done on 2026-11-15 for 0.4.0).

1. Push the change to `main`.
2. Start the workflow:

   ```sh
   gh workflow run pages.yml
   gh run watch
   ```

The site is then at <https://ch3w3y.github.io/chewvy/>.

The workflow runs on a GitHub-hosted runner, as the `macos` workflow does. It
is started by hand only and has no `pull_request` trigger. The `release` and
`ci` workflows are disabled in this repository. That does not affect this one.

## Publish from a branch instead

Use this path if you prefer not to add a workflow.

1. Put the files on an orphan `gh-pages` branch:

   ```sh
   git worktree add --orphan -b gh-pages ../chewvy-pages
   cp -r site/index.html site/credits.html site/styles.css site/main.js site/assets ../chewvy-pages/
   touch ../chewvy-pages/.nojekyll
   git -C ../chewvy-pages add -A
   git -C ../chewvy-pages commit -m "Website for Chewvy X.Y.Z"
   git -C ../chewvy-pages push -u origin gh-pages
   ```

2. Turn Pages on for that branch. In **Settings > Pages**, set **Source** to
   **Deploy from a branch**, and choose `gh-pages` and `/ (root)`. The same
   from a terminal:

   ```sh
   gh api -X POST repos/Ch3w3y/chewvy/pages -f 'source[branch]=gh-pages' -f 'source[path]=/'
   ```

To update the site later, copy the files into the worktree again, commit and
push.

## After you publish

1. Open <https://ch3w3y.github.io/chewvy/> on a desktop and on a phone.
2. Check that the download buttons point at the new release's packages.
3. In **Settings > Pages**, turn on **Enforce HTTPS** if it is not on.
4. Optionally set the repository's website field:
   `gh repo edit Ch3w3y/chewvy --homepage https://ch3w3y.github.io/chewvy/`.

`README.md` links to the site's address. Until the site is published, that
link answers 404.

## What is published

Only `index.html`, `credits.html`, `styles.css`, `main.js` and `assets/`. The
tools, this file and `deploy/` stay in the repository.
