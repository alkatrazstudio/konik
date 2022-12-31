# {{title}} v{{version}}

{{description}}


## Playing music

```
{{name}} <path_1> <path_2> ...
```

where `<path_N>` is a path to a music file or a folder.

The folders are loaded recursively.

Supported file formats: FLAC, OGG, MP3.

{{title}} also supports CUE sheets.


## Hot keys

* `NumPad 5` - play/stop
* `NumPad 0` - play/pause
* `NumPad 4` - previous track
* `NumPad 6` - next track
* `NumPad 7` - previous folder
* `NumPad 9` - next folder
* `NumPad 2` - decrease volume
* `NumPad 8` - increase volume
* `NumPad 1` - decrease system volume
* `NumPad 3` - increase system volume


## ListenBrainz and Last.fm

{{title}} supports scrobbling tracks via ListenBrainz or Last.fm.
You need to authenticate your account via the following commands:

* `{{name}} listenbrainz-auth` - authenticate your ListenBrainz account
* `{{name}} lastfm-auth` - authenticate your Last.fm account


## Tray context menu

* **Show current file** - open the default file manager and highlight the current file
* **Exit** - close Konik


## More info

Run `{{name}} help`, `{{name}} version`
or visit the homepage `{{homepage}}`
