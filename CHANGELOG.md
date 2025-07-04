# Konik - CHANGELOG


## v0.3.3 (June 29, 2025)

Fixed: wrong messages in the logs about overlapping network requests
Changed: do not log useless notification warnings


## v0.3.2 (June 22, 2025)

Fixed: application hangs on a scrobble request if the previous one is still running
Fixed: missing User-Agent in GET requests
Changed: limit the duration of the network requests


## v0.3.1 (June 12, 2025)

- Fixed: tray tooltip
- Fixed: wrong ListenBrainz submits for tracks with existing but empty album name
- Fixed: non-submittable listens from the ListenBrainz submit queue stuck in the file forever
- Changed: increased batch submit size for ListenBrainz
- Changed: playlist end is no longer logged as an error


## v0.3.0 (September 15, 2024)

- Added: submit track duration to ListenBrainz
- Fixed: showing unreadable/broken tags
- Fixed: passing relative paths to a running instance
- Fixed: using wrong ListenBrainz field for the music player name


## v0.2.0 (May 25, 2024)

- Fixed: sending invalid payload to ListenBrainz
- Added: MPRIS controls for volume


## v0.1.4 (September 3, 2023)

- Fixed: various bugs when the tags cannot be retrieved
- Improved: start and end of startup and shutdown are now clearly marked in the terminal output


## v0.1.3 (July 9, 2023)

- Fixed: system volume is stuck when 1% volume step is not supported
- Fixed: can't modify the system volume after changing the current playback device


## v0.1.2 (February 19, 2023)

- Fixed: crashing on short tracks


## v0.1.1 (January 7, 2023)

- Changed: volume step is now 1%
- Improved: exit is now instantaneous


## v0.1.0 (January 1, 2023)

- Initial release
