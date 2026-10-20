# Store declarations

Text to paste into the store consoles when a build adds or changes a sensitive
permission. Keep it in sync with what the apps actually do.

## Google Play: Photo and Video Permissions (attach drawer)

Play Console → App content → **Photo and video permissions**. Needed because the
Android app declares `READ_MEDIA_IMAGES`, `READ_MEDIA_VIDEO` and
`READ_MEDIA_VISUAL_USER_SELECTED`.

**Does your app use these permissions for its core functionality?** Yes.

**Describe your app's core functionality and why it needs broad access to photos
and videos**:

- `READ_MEDIA_IMAGES`: Caper is a messaging app. Sharing photos in chats is a
  core, frequent action. Its attach drawer shows recent photos in-line beside
  the conversation so people can tap several in a row while they keep chatting.
  Nothing uploads until tapped.
- `READ_MEDIA_VIDEO`: The same in-line attach drawer lists recent videos beside
  photos, with durations, so people can share clips in a conversation without
  leaving it. Only videos the person taps are uploaded; others are never read
  beyond their thumbnail.

**Why the system photo picker is not sufficient:** Caper's composer keeps the
chat visible while a half-height drawer shows recent media. Each tap adds the
item to the message and starts its upload at once. The drawer numbers items in
order, and it shows and enforces the 10-attachment limit as people choose. The
system picker is a separate full-screen step that returns all items only at
the end.

Notes for reviewers:

- Access is requested only when the person opens the attach drawer, never at
  launch. On Android 14+ the system's "Select photos and videos" choice is
  fully supported (the drawer shows only what was shared, with a "Manage" link).
- If access is denied, the drawer offers the system photo picker instead, so the
  feature works without the permission.
- No photos or videos are read in the background, uploaded without a tap, or
  used for anything other than the message being written.

If Play rejects the declaration, delete the three `READ_MEDIA_*` lines from
`apps/native/android/app/src/main/AndroidManifest.xml` (keep
`READ_EXTERNAL_STORAGE` with `maxSdkVersion="32"` or remove it too). The drawer
then works through the system photo picker on Android 13+.

## App Store / TestFlight

There is no form to fill in. iOS shows these purpose strings (`project.yml`)
when it asks:

- Photo library: "Caper shows your recent photos and videos so you can attach
  them to messages. Only items you choose are uploaded."
- Camera: "Caper uses your camera when you take a photo or video to attach to a
  message."
- Microphone: "Caper uses your microphone when you join a voice channel,
  record a voice message or record a video to attach."

App Store Connect → App Privacy: if "Photos or Videos" is not already declared,
add it as **User Content → Photos or Videos**, collected, linked to the user's
account, used for **App Functionality**, not for tracking. Sent attachments
already reach Caper's servers, so this may already be declared.

Voice messages (October 2026) upload recorded audio and send it to an AI
provider for a transcript. If "Audio Data" is not already declared, add it as
**User Content → Audio Data**, collected, linked to the user's account, used
for **App Functionality**, not for tracking.
