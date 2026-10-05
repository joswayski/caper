#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";

const manifestPath = new URL("../apps/native/android/app/src/main/AndroidManifest.xml", import.meta.url);
const manifest = readFileSync(manifestPath, "utf8");
const start = "        <!-- BEGIN generated launcher aliases; run scripts/generate-android-launcher-aliases.mjs -->";
const end = "        <!-- END generated launcher aliases -->";

// Retain old component names so enabled aliases and pinned shortcuts survive upgrades.
// Every entry uses the original mascot; the app no longer switches components.
const aliases = [
  `        <activity-alias
            android:name="chat.caper.android.launcher.Default"
            android:enabled="true"
            android:exported="true"
            android:icon="@drawable/ic_caper_app"
            android:label="Caper (Development)"
            android:targetActivity="chat.caper.android.MainActivity">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity-alias>`,
  ...Array.from({ length: 800 }, (_, index) => `        <activity-alias
            android:name="chat.caper.android.launcher.Avatar${index}"
            android:enabled="false"
            android:exported="true"
            android:icon="@drawable/ic_caper_app"
            android:label="Caper (Development)"
            android:targetActivity="chat.caper.android.MainActivity">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity-alias>`),
].join("\n");

const replacement = `${start}\n${aliases}\n${end}`;
const pattern = new RegExp(`${start.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[\\s\\S]*?${end.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`);
if (!pattern.test(manifest)) throw new Error("Generated launcher alias markers are missing from AndroidManifest.xml");
const generated = manifest.replace(pattern, replacement);
if (process.argv.includes("--check")) assert.equal(manifest, generated, "Android launcher alias drift");
else writeFileSync(manifestPath, generated);
console.log(`${process.argv.includes("--check") ? "Verified" : "Exported"} the original mascot for the default launcher and all 800 legacy aliases.`);
