import { createFileRoute } from "@tanstack/react-router";
import type { ReactNode } from "react";
import Wordmark from "../components/Wordmark";

// Mailbox that answers privacy and account-deletion requests.
const CONTACT = "privacy@caper.chat";
const UPDATED = "November 22, 2026";

export const Route = createFileRoute("/privacy")({
  head: () => ({ meta: [{ title: "Privacy — Caper" }] }),
  component: Privacy,
});

function Privacy() {
  return (
    <main className="grid min-h-dvh justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-16 max-[480px]:px-5 max-[480px]:pt-8">
      <article className="w-full max-w-[640px] leading-[1.7] text-content-muted">
        <Wordmark />
        <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] max-[480px]:mt-[42px]">
          UPDATED {UPDATED.toUpperCase()}
        </p>
        <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.025em] text-content">
          Privacy
        </h1>
        <p>
          Caper is a place to text and talk with your people. This page explains what we collect to run it, and what we
          don’t. It applies to caper.chat and the Caper apps for Mac, Windows, Linux, iPhone, and Android.
        </p>

        <Section title="What we collect">
          <ul className="list-disc space-y-2 pl-5">
            <li>
              <b className="text-content">Your email address</b>, when you sign in. We use it only to send one-time
              sign-in codes and a welcome note.
            </li>
            <li>
              <b className="text-content">Your profile</b>: your username, display name, current profile picture, and
              any previous profile pictures you keep in avatar history, plus icons for spaces you own.
            </li>
            <li>
              <b className="text-content">Messages and interactions</b>: channel messages, direct messages, threads,
              reactions, and pins, stored so people in those conversations can read them.
            </li>
            <li>
              <b className="text-content">Files you attach</b>, stored so people in that conversation can open them.
              Photos are re-encoded on your device before upload, which removes embedded details such as location.
            </li>
            <li>
              <b className="text-content">Voice messages and dictation</b>: voice messages and their automatic
              transcripts are stored with the conversation. Dictation audio is sent for transcription and not retained
              as audio after the text is returned.
            </li>
            <li>
              <b className="text-content">Your spaces and channels</b>: what you create, join, and who you invite.
            </li>
            <li>
              <b className="text-content">A notification token for your phone</b>, if you allow notifications on iPhone
              or Android. It lets us send you message alerts, and we stop using it when you log out on that phone.
            </li>
            <li>
              <b className="text-content">Basic service logs</b>, such as errors and connection timings, used to keep
              Caper working. Sign-in requests record a one-way hash of your IP address to limit abuse.
            </li>
          </ul>
        </Section>

        <Section title="Voice">
          <p>
            Voice calls travel through Cloudflare’s real-time network to the other people in the call. We don’t record
            calls. The microphone test in audio settings runs on your device and isn’t uploaded.
          </p>
        </Section>

        <Section title="What we don’t do">
          <p>
            We don’t sell your data or run third-party tracking analytics. Caper sets a single cookie to keep you signed
            in. If you open the GIF and sticker picker, your device connects directly to KLIPY, which may include
            sponsored GIF results on mobile under KLIPY’s own privacy policy.
          </p>
        </Section>

        <Section title="Who handles data for us">
          <ul className="list-disc space-y-2 pl-5">
            <li>Amazon Web Services hosts the Caper API, gateway, and web app, and sends sign-in emails.</li>
            <li>PlanetScale hosts the PostgreSQL database.</li>
            <li>
              Cloudflare carries voice and network traffic, stores attached files and avatars, and fetches link
              previews.
            </li>
            <li>AI transcription providers (via OpenRouter) transcribe voice messages and dictation.</li>
            <li>KLIPY serves GIFs and stickers when you open the picker.</li>
            <li>Axiom stores service logs.</li>
            <li>
              Apple and Google deliver phone notifications, which include the sender and a short preview of the message.
            </li>
          </ul>
        </Section>

        <Section title="How long we keep it">
          <p>
            Messages and your account stay until they’re deleted. Attached files are removed from storage about a day
            after they’re deleted; uploads that are never sent are removed within a day. Leaving a space or channel, or
            being removed from one, ends your access but keeps the record. We keep a history of your sign-ins; one-time
            sign-in codes are removed after a week.
          </p>
        </Section>

        <Section title="Your choices">
          <p>
            You can change your username, display name, and profile picture at any time. To get a copy of your data,
            email{" "}
            <a
              className="rounded-[2px] text-content underline underline-offset-4 transition-colors hover:text-terracotta-bright focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4"
              href={`mailto:${CONTACT}`}
            >
              {CONTACT}
            </a>{" "}
            from the address you sign in with. To delete your account, see below.
          </p>
        </Section>

        <Section title="Delete your account" id="delete-account">
          <p>
            Email{" "}
            <a
              className="rounded-[2px] text-content underline underline-offset-4 transition-colors hover:text-terracotta-bright focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4"
              href={`mailto:${CONTACT}?subject=Delete%20my%20Caper%20account`}
            >
              {CONTACT}
            </a>{" "}
            from the address you sign in with and ask us to delete your Caper account. Privacy settings in every Caper
            app link here. We’ll reply to confirm, then within 30 days:
          </p>
          <ul className="mt-3 list-disc space-y-2 pl-5">
            <li>remove your email address, username, and display name from your account;</li>
            <li>sign you out everywhere and stop sending you notifications;</li>
            <li>remove you from your spaces and channels.</li>
          </ul>
          <p className="mt-3">
            Like other chat apps, some things stay so the people you talked with keep their history. Messages and files
            you sent stay in their conversations, shown as from a deleted account and no longer linked to your email,
            username, or display name, for as long as those conversations exist. We keep a record of past sign-ins, with
            the one-way IP hash above, for security and to prevent abuse. If you own spaces, tell us whether to delete
            them or hand them to another member.
          </p>
        </Section>

        <Section title="Children">
          <p>Caper isn’t meant for children under 13, and we don’t knowingly collect their data.</p>
        </Section>

        <Section title="Changes">
          <p>If this policy changes, we’ll update this page and the date above.</p>
        </Section>
      </article>
    </main>
  );
}

function Section({ title, id, children }: Readonly<{ title: string; id?: string; children: ReactNode }>) {
  return (
    <section className="mt-10 scroll-mt-6" id={id}>
      <h2 className="mb-3 text-[1.15rem] font-bold tracking-[-.02em] text-content">{title}</h2>
      {children}
    </section>
  );
}
