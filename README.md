# Toot
Toot is a Mastodon client for COSMIC.

## Dependencies
- libsecret-1-dev

- libgstreamer1.0-dev
- libgstreamer-plugins-base1.0-dev

Video playback uses GStreamer. Install the playback, network, and codec plugins
for the formats served by your instance. On Ubuntu/Pop!_OS:

```sh
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-libav
```

Open a post's video thumbnail to play it in the attachment viewer. Use the
play/pause button (or Space), seek bar, and mute button to control playback.
Left/Right move between attachments and Escape closes the viewer. Animated
video attachments loop and start muted. Playback stops when you leave the video.
If playback fails, the open-in-browser button still opens the original media.
