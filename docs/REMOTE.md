# Remote conversion over SSH

Remote control lets a Windows client submit a movie queue to a Mac running Movie Harbor. Both computers access the same existing media share. Nothing is uploaded by Movie Harbor.

## On the Mac

1. Install FFmpeg/ffprobe and open Movie Harbor.
2. In **Settings → Remote conversion**, choose **Enable remote control**.
3. Select the folder that remote clients may access. Keep it as narrow as practical: a media folder, rather than your home directory.
4. Reveal and copy the session access token. Keep it private.
5. Enable macOS Remote Login for the intended SSH account and configure its public key using your normal SSH setup.

The control server listens only on `127.0.0.1:38473`. It is off until enabled, stops when the app closes, and generates a new 256-bit token each time it is enabled. Settings and tokens are not persisted by Movie Harbor. Disabling hosting stops further remote requests; an already-started conversion remains in the Mac's queue.

## On Windows

1. Verify SSH key authentication to your Mac using the system SSH client. Confirm the server's host key through a trusted channel and add it to your known-hosts file. Movie Harbor requires a previously trusted host; it does not accept new host keys automatically.
2. Mount the same media share and open Movie Harbor.
3. In **Settings → Remote conversion**, enter the Mac hostname and SSH username, select your private-key file, select the corresponding shared folder on Windows, and paste the Mac's access token.
4. Choose **Connect to Mac**. The app creates an SSH tunnel on an available local port. The Mac must already be running with hosting enabled.
5. Add movies from the configured shared folder. Their relative paths are mapped to the folder selected on the Mac. The Mac performs inspection, encoding, checks, and publication.

Example: selecting a folder containing `Movies/Example.mkv` on each computer means that the relative path `Movies/Example.mkv` resolves to the same movie. No fixed drive letter, NAS address, username, or key filename is assumed.

Remote clients may request inspection, queue status, conversion, and cancellation. They cannot set an arbitrary executable or issue shell commands through the control protocol. The server resolves paths and rejects paths outside its shared folder, including symlink escapes. A local queue containing sources outside that folder is not exposed to remote clients.

Closing the Windows client closes its tunnel; jobs already running on the Mac continue. Reconnect with the same token while the Mac session remains enabled. The Mac queue is held in memory, so quitting the Mac app ends that session.

SSH protects the network connection; the access token authenticates the application session. The key is passed to the installed SSH client and is never uploaded to GitHub or sent to the Mac as a file. Use only private-key files you control. Hostnames currently accept DNS names and IPv4 addresses; IPv6 literals and custom SSH-port controls are not exposed in the UI.
