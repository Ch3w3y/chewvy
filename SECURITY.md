# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private
reporting instead: on the repository page choose **Security**, then
**Report a vulnerability**. You will get a reply there.

Useful reports say what an attacker needs (network position, a malicious
server, a crafted media file, local access), what they gain, and how to
reproduce it. Please do not include a real token or password; describe where
it leaked instead.

## What counts

Chewvy holds bearer tokens for a Plex account and for Jellyfin servers, talks
to servers the user chooses, and hands media to mpv and FFmpeg. Examples of
what we want to hear about:

- a token or password reaching a log, an error message, a URL, a process
  argument or a file other than the token store;
- a token being sent to a host it does not belong to;
- a malicious server response causing a crash, a write outside the app's
  config or cache directories, or code execution;
- a problem in `install.sh` or the Windows installer that lets something other
  than the published, checksummed package be installed.

Vulnerabilities in mpv, FFmpeg or another bundled library should go to that
project; tell us too, so a release can pick up the fix.

## Supported versions

Only the latest release receives fixes.

## Things to know as a user

- The release packages are **not code-signed**. Verify a download against the
  `SHA256SUMS` file published with the release; `install.sh` does this for you.
- On Linux, tokens are kept in the system keyring (Secret Service). If no
  keyring is running, Chewvy asks before falling back to a file that only your
  user can read but that is not encrypted.
- A Jellyfin password, if you use one, is sent once to your own server to
  obtain a token and is never stored. Use HTTPS or a trusted network for that
  server.
