import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = path => readFileSync(new URL(`../${path}`, import.meta.url));
const desktop = 'apps/native/desktop/resources/';
const mac = 'apps/native/apple/Resources/CaperIcons.xcassets/AppIcon.appiconset/';
const pngSize = bytes => {
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
  assert.equal(bytes[24], 8, '8-bit channels');
  assert.equal(bytes[25], 6, 'Truecolor RGBA, not a flattened or palette icon');
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
};

test('Linux installed icon uses the canonical main SVG', () => {
  assert.deepEqual(read(`${desktop}caper.svg`), read('apps/web/public/caper-face.svg'));
});

test('macOS AppIcon catalog maps every 1x/2x slot to the correctly sized RGBA PNG', () => {
  const catalog = JSON.parse(read(`${mac}Contents.json`));
  const images = catalog.images.filter(image => image.idiom === 'mac');
  assert.equal(images.length, 10);
  assert.deepEqual([...new Set(images.map(image => image.filename))].sort(),
    [16, 32, 64, 128, 256, 512, 1024].map(size => `icon-${size}.png`).sort());
  for (const image of images) {
    const [width, height] = image.size.split('x').map(Number);
    const scale = Number.parseInt(image.scale, 10);
    assert.deepEqual(pngSize(read(`${mac}${image.filename}`)), [width * scale, height * scale]);
  }
});

test('Windows ICO frames and Linux window PNG share the exact macOS raster exports', () => {
  const ico = read(`${desktop}caper.ico`);
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  assert.equal(ico.readUInt16LE(0), 0);
  assert.equal(ico.readUInt16LE(2), 1);
  assert.equal(ico.readUInt16LE(4), sizes.length);
  let offset = 6 + 16 * sizes.length;
  for (const [index, size] of sizes.entries()) {
    const entry = 6 + 16 * index;
    assert.equal(ico[entry] || 256, size);
    assert.equal(ico[entry + 1] || 256, size);
    assert.equal(ico.readUInt16LE(entry + 4), 1);
    assert.equal(ico.readUInt16LE(entry + 6), 32);
    assert.equal(ico.readUInt32LE(entry + 12), offset);
    const length = ico.readUInt32LE(entry + 8);
    const png = ico.subarray(offset, offset + length);
    assert.deepEqual(pngSize(png), [size, size]);
    if (![24, 48].includes(size)) assert.deepEqual(png, read(`${mac}icon-${size}.png`));
    offset += length;
  }
  assert.equal(offset, ico.length, 'No truncated or unused payloads');
  assert.deepEqual(read(`${desktop}caper-icon.png`), read(`${mac}icon-128.png`));
});

test('desktop build uses the Windows small/big icon patch and ships its license', () => {
  const native = 'apps/native/desktop/';
  assert.match(read(`${native}Cargo.toml`).toString(), /\[patch\.crates-io\]\s+egui-winit = \{ path = "vendor\/egui-winit" \}/);
  const lock = read(`${native}Cargo.lock`).toString().split('[[package]]')
    .find(entry => entry.includes('name = "egui-winit"'));
  assert.ok(lock);
  assert.match(lock, /version = "0\.33\.3"/);
  assert.doesNotMatch(lock, /source = "registry/);
  const source = read(`${native}vendor/egui-winit/src/lib.rs`).toString();
  const iconArm = source.split('ViewportCommand::Icon(icon) => {')[1].split('ViewportCommand::IMERect')[0];
  assert.match(iconArm, /#\[cfg\(target_os = "windows"\)\]\s+winit::platform::windows::WindowExtWindows::set_taskbar_icon\(window, winit_icon\.clone\(\)\);/);
  assert.match(iconArm, /window\.set_window_icon\(winit_icon\);/);
  assert.ok(read(`${native}THIRD-PARTY-NOTICES.md`).toString()
    .includes(read(`${native}vendor/egui-winit/LICENSE-MIT`).toString().trim()));
});

test('Wayland application ID matches the installed desktop entry and themed icon name', () => {
  const entry = read(`${desktop}caper.desktop`).toString();
  assert.match(entry, /^Icon=caper$/m);
  assert.match(read('apps/native/desktop/src/main.rs').toString(), /#\[cfg\(target_os = "linux"\)\]\s+let viewport = viewport\.with_app_id\("caper"\);/);
  assert.match(read('apps/native/desktop/src/app_icon.rs').toString(), /directory\.join\("caper\.svg"\)/);
});
