#!/usr/bin/env bash
# Regenerates the metadata-cleaning fixtures every client checks against:
# `inputs/` (small files carrying location, names, serials and titles) and
# `cases.json` (declared type and the SHA-256 of the cleaned output, or the
# refusal). Expected outputs come from the web cleaner and are accepted only
# after ExifTool finds no identifying tag left and FFmpeg/ImageMagick still
# decode them. Needs ffmpeg (with libx264, libvpx, libvorbis, libopus,
# libspeex, libtheora), exiftool, ImageMagick 7 and Node 24.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
in="$here/inputs"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
rm -rf "$in"
mkdir -p "$in"
cd "$work"

ff() { ffmpeg -nostdin -hide_banner -loglevel error -y "$@"; }
tag() {
  exiftool -q -overwrite_original -GPSLatitude=40.7 -GPSLatitudeRef=N -GPSLongitude=74 -GPSLongitudeRef=W \
    -Artist=Secret -SerialNumber=SN123 -OwnerName=Secret -XMP:Creator=Secret "$@"
}

magick -size 16x12 gradient:red-blue base.png
ff -f lavfi -i sine=frequency=440:duration=0.3 -ac 1 -ar 8000 tone.wav
ff -f lavfi -i testsrc=size=32x24:duration=0.4:rate=5 -pix_fmt yuv420p video.y4m

# Photos
magick base.png "$in/photo.jpg" && tag -Orientation#=6 -Comment=Secret "$in/photo.jpg"
magick base.png "$in/no-jfif.jpg" && tag -Orientation#=8 -JFIF:all= "$in/no-jfif.jpg"
cp "$in/photo.jpg" "$in/trailer.jpg" && printf 'SECRET-TRAILER' >>"$in/trailer.jpg"
magick base.png -interlace JPEG "$in/progressive.jpg" && tag "$in/progressive.jpg"
cp base.png "$in/photo.png" && tag -Orientation#=6 -PNG:Comment=Secret "$in/photo.png"
magick base.png -loop 0 "$in/photo.gif" && exiftool -q -overwrite_original -XMP:Creator=Secret -Comment=Secret "$in/photo.gif"
magick base.png "$in/photo.webp" && tag -Orientation#=3 "$in/photo.webp"
magick base.png -define webp:lossless=true "$in/plain.webp" && tag "$in/plain.webp"
magick base.png "$in/photo.heic" && tag "$in/photo.heic"
magick base.png "$in/photo.avif" && tag "$in/photo.avif"
magick base.png -compress none "$in/photo.tif" && tag -Make=Camera "$in/photo.tif"
magick base.png -compress none -endian MSB "$in/big-endian.tif" && tag "$in/big-endian.tif"
magick base.png "$in/photo.psd" && tag "$in/photo.psd"
magick base.png "$in/photo.jp2" && tag "$in/photo.jp2"

# Real camera maker notes (serials, owner), grafted from CC0 raw.pixls.us
# samples into small TIFFs.
raw() {
  curl -fsSL -o "$2" "https://raw.pixls.us/data/$1"
  echo "$3  $2" | sha256sum --check --quiet
  magick base.png -compress none "$in/maker-$4.tif"
  exiftool -q -q -overwrite_original -tagsfromfile "$2" -makernotes -make -model "$in/maker-$4.tif"
  tag "$in/maker-$4.tif"
}
raw "Canon/EOS%20350D/IMG_1707.CR2" canon.cr2 8cbb84e04d93b005fe082da9c954122a612b5281af00aa088d767850f343fd38 canon
raw "Nikon/D50/DSC_5155.NEF" nikon.nef 9f94c27672a63f946c92d43910ef0c266dffaa14dae0007ae00ac72d2c684935 nikon
raw "Olympus/E-1/E_1__C106743_gredos.ORF" olympus.orf 042286653fbae5b085bef4a4e626145385ea6e82e817f166b4904abcef64457c olympus
raw "Panasonic/DMC-G1/RAW_PANASONIC_G1.RW2" panasonic.rw2 aa3b92d6592744a889d7ea50aaf3ea55e0ec961d04c740d437d3396ca79757be panasonic
raw "Pentax/K-01/_IGP2068.DNG" pentax.dng 4c9aac2ce2720154616ad7678385d1ec1f626d6464a1d5c6a5a4d821a78a608d pentax

# Audio
ff -i tone.wav -metadata artist=Secret -metadata title=Secret -write_id3v1 1 "$in/tone.mp3"
ff -i tone.wav -c:a aac -metadata artist=Secret -metadata location=+40.7-074.0/ "$in/tone.m4a"
ff -i tone.wav -i base.png -map 0 -map 1 -c:a aac -c:v png -disposition:v attached_pic -metadata artist=Secret "$in/cover.m4a"
ff -i tone.wav -c:a aac -f adts adts.aac
node -e '
const fs = require("node:fs");
const frame = Buffer.concat([Buffer.from("TPE1"), Buffer.from([0, 0, 0, 7, 0, 0, 3]), Buffer.from("Secret")]);
const body = Buffer.concat([frame, Buffer.alloc(16)]);
const n = body.length;
const header = Buffer.from([0x49, 0x44, 0x33, 4, 0, 0, (n >> 21) & 127, (n >> 14) & 127, (n >> 7) & 127, n & 127]);
fs.writeFileSync(process.argv[1], Buffer.concat([header, body, fs.readFileSync("adts.aac")]));
' "$in/tone.aac"
ff -i tone.wav -metadata artist=Secret -metadata comment=Secret "$in/tone.flac"
ff -i tone.wav -c:a libvorbis -metadata artist=Secret "$in/tone.ogg"
ff -i tone.wav -c:a libopus -metadata artist=Secret "$in/tone.opus"
ff -i tone.wav -c:a flac -metadata artist=Secret -f ogg "$in/flac.oga"
ff -i tone.wav -c:a libspeex -metadata artist=Secret "$in/tone.spx"
ff -i tone.wav -metadata artist=Secret -metadata comment=Secret "$in/tone.wav"
ff -i tone.wav -c:a pcm_s16be -metadata artist=Secret -write_id3v2 1 "$in/tone.aiff"

# Video
ff -i video.y4m -i tone.wav -shortest -c:v libx264 -c:a aac -metadata title=Secret \
  -metadata location=+40.7-074.0/ -movflags +use_metadata_tags "$in/clip.mp4"
ff -i video.y4m -c:v libx264 -metadata title=Secret -metadata location=+40.7-074.0/ "$in/clip.mov"
printf '1\n00:00:00,000 --> 00:00:00,300\nSecret\n' >subtitle.srt
ff -i video.y4m -i subtitle.srt -c:v libx264 -c:s mov_text -metadata title=Secret "$in/subtitled.mp4"
ff -i video.y4m -i tone.wav -shortest -c:v libvpx -c:a libvorbis -metadata title=Secret \
  -metadata:s:v title=Secret "$in/clip.webm"
ff -i video.y4m -i subtitle.srt -c:v libvpx -c:s webvtt -metadata title=Secret "$in/subtitled.webm"
ff -i video.y4m -c:v libtheora -metadata title=Secret "$in/clip.ogv"
ff -i video.y4m -i tone.wav -shortest -c:v mpeg4 -c:a pcm_s16le -metadata title=Secret -metadata artist=Secret "$in/clip.avi"

# Formats with nothing to clean, and refusals
printf '\xff\x0a\xfa\x7f\x01\x00' >"$in/bare.jxl"
printf '%%PDF-1.7\n%%Secret\n' >"$in/document.pdf"
magick base.png "$in/picture.bmp"
printf '<svg xmlns="http://www.w3.org/2000/svg"/>' >"$in/picture.svg"
printf '\0\0\0\x18ftypcrx \0\0\0\x01crx isom' >"$in/canon.cr3"
head -c 200 "$in/photo.png" >"$in/truncated.png"

node --experimental-transform-types --no-warnings "$here/cases.ts" "$repo/apps/web/src/chat/metadata.ts" "$here"
