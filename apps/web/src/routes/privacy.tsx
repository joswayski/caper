import { createFileRoute } from "@tanstack/react-router";
import type { ReactNode } from "react";
import Wordmark from "../components/Wordmark";

// Mailbox that answers privacy and account-deletion requests.
const CONTACT = "privacy@caper.chat";
const UPDATED = "October 1, 2026";

export const Route = createFileRoute("/privacy")({
  head: () => ({ meta: [{ title: "Privacy - Caper" }] }),
  component: Privacy,
});

function Privacy() {
  return <main className="grid min-h-dvh justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-16 max-[480px]:px-5 max-[480px]:pt-8">
    <article className="w-full max-w-[640px] leading-[1.7] text-content-muted">
      <Wordmark />
      <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] max-[480px]:mt-[42px]">UPDATED {UPDATED.toUpperCase()}</p>
      <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em] text-content">Privacy</h1>
      <p>Caper is a place to text and talk with your people. This page explains what we collect to run it, and what we don't. It applies to caper.chat and the Caper apps for Mac, Windows, Linux, iPhone and Android.</p>

      <Section title="What we collect">
        <ul className="list-disc space-y-2 pl-5">
          <li><b className="text-content">Your email address</b>, if you sign in. We use it only to send you one-time sign-in codes.</li>
          <li><b className="text-content">Your username and display name</b>, which other people see in conversations.</li>
          <li><b className="text-content">Messages you send</b>, stored so the people in that channel can read them.</li>
          <li><b className="text-content">Your spaces and channels</b>: what you create, join, and who you add.</li>
          <li><b className="text-content">Your approximate country</b>, taken from your connection when you join voice, and shown as a flag next to your name.</li>
          <li><b className="text-content">Basic service logs</b>, such as errors and connection timings, used to keep Caper working. Sign-in requests record a one-way hash of your IP address to limit abuse.</li>
        </ul>
      </Section>

      <Section title="Voice">
        <p>Voice calls travel through Cloudflare's real-time network to the other people in the call. We don't record calls. The microphone test in audio settings runs on your device and isn't uploaded.</p>
      </Section>

      <Section title="What we don't do">
        <p>We don't sell your data, show ads, or use third-party trackers. Caper sets a single cookie to keep you signed in.</p>
      </Section>

      <Section title="Who handles data for us">
        <ul className="list-disc space-y-2 pl-5">
          <li>Amazon Web Services hosts Caper and sends sign-in emails.</li>
          <li>Cloudflare carries voice and network traffic.</li>
          <li>Axiom stores service logs.</li>
        </ul>
      </Section>

      <Section title="How long we keep it">
        <p>We keep your account, messages, spaces and sign-in history until you ask us to delete them. Signing out, leaving a space, or being removed from one ends access but keeps the record.</p>
      </Section>

      <Section title="Your choices">
        <p>You can change your username and display name at any time. To get a copy of your data or delete your account and messages, email <a className="text-content underline underline-offset-4" href={`mailto:${CONTACT}`}>{CONTACT}</a> from the address you sign in with.</p>
      </Section>

      <Section title="Children">
        <p>Caper isn't meant for children under 13, and we don't knowingly collect their data.</p>
      </Section>

      <Section title="Changes">
        <p>If this policy changes, we'll update this page and the date above.</p>
      </Section>
    </article>
  </main>;
}

function Section({ title, children }: Readonly<{ title: string; children: ReactNode }>) {
  return <section className="mt-10">
    <h2 className="mb-3 text-[1.15rem] font-bold tracking-[-.02em] text-content">{title}</h2>
    {children}
  </section>;
}
