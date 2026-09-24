package chat.caper.android.voice

import chat.caper.android.data.ApiException
import chat.caper.android.model.SessionDescription
import chat.caper.android.model.SignalResponse
import chat.caper.android.model.SignalTrack
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.Assert.*
import org.junit.Test
import org.webrtc.SessionDescription as RtcSessionDescription

class VoiceEngineLifecycleTest {
    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `local mute and deafen silence tracks before blocked signaling`() = runTest {
        val signaling = Mutex()
        val synced = mutableListOf<Pair<Boolean, Boolean>>()
        var microphoneEnabled = false
        var remoteEnabled = true
        val local = VoiceLocalMute(signaling, { synced += microphoneEnabled to remoteEnabled }) { muted, deafened ->
            microphoneEnabled = !muted
            remoteEnabled = !deafened
        }
        local.setMuted(false)
        assertTrue(microphoneEnabled)
        signaling.lock() // A roster/ICE operation holds the signaling mutex.
        val mute = launch { local.setMuted(true) }
        runCurrent()
        assertFalse(microphoneEnabled)
        assertEquals(1, synced.size)
        val deafen = launch { local.setDeafened(true) }
        runCurrent()
        assertFalse(remoteEnabled)
        assertTrue(local.muted)
        assertEquals(1, synced.size)
        signaling.unlock()
        advanceUntilIdle()
        mute.join(); deafen.join()
        assertEquals(3, synced.size)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `departed remote track is silent during close and transient failure preserves MID for retry`() = runTest {
        var audible = true
        var remembered = true
        val response = CompletableDeferred<Unit>()
        val closing = launch {
            closeDepartedSubscription({ audible = false }, { response.await() }, { remembered = false })
        }
        runCurrent()
        assertFalse(audible)
        assertTrue(remembered)
        response.completeExceptionally(ApiException(503, "temporary"))
        closing.join()
        assertTrue(remembered)
        closeDepartedSubscription({ audible = false }, {}, { remembered = false })
        assertFalse(remembered)

        var touchedDisposedTrack = false
        assertFalse(acceptRemoteCallback(true, { touchedDisposedTrack = true }, { false }))
        assertFalse(touchedDisposedTrack)
        assertFalse(acceptRemoteCallback(false, { audible = false }, { false }))
        assertFalse(audible)
    }

    @Test fun `terminal mute failure tears down local audio rather than reporting a warning`() {
        val gate = VoiceResourceGate()
        var microphoneEnabled = true
        var warning = false
        handleVoiceControlError(ApiException(403, "membership denied"),
            terminal = { gate.close { microphoneEnabled = false } },
            transient = { warning = true },
        )
        assertFalse(microphoneEnabled)
        assertFalse(gate.isOpen)
        assertFalse(warning)
        assertThrows(IllegalStateException::class.java) { gate.use { microphoneEnabled = true } }
        handleVoiceControlError(ApiException(503, "temporary"),
            terminal = { fail("503 must not end a call") }, transient = { warning = true },
        )
        assertTrue(warning)
        assertFalse(transientVoiceControlError(ApiException(401, "expired")))
        assertFalse(transientVoiceControlError(ApiException(404, "missing session")))
        assertFalse(transientVoiceControlError(ApiException(502, "bad SDP", "ice_restart_invalid")))
        assertTrue(transientVoiceControlError(ApiException(403, "provider", "ice_restart_retry")))
    }

    @Test fun `pending SDP is accepted without waiting for optional ICE gathering`() {
        val offer = RtcSessionDescription(RtcSessionDescription.Type.OFFER, "v=0\r\na=ice-ufrag:pending\r\n")
        assertEquals(offer.description, validatedLocalSdp(offer, RtcSessionDescription.Type.OFFER))
        assertThrows(IllegalStateException::class.java) { validatedLocalSdp(offer, RtcSessionDescription.Type.ANSWER) }
        assertThrows(IllegalStateException::class.java) {
            validatedLocalSdp(RtcSessionDescription(RtcSessionDescription.Type.OFFER, ""), RtcSessionDescription.Type.OFFER)
        }
        assertThrows(IllegalArgumentException::class.java) { validatedLocalSdp(null, RtcSessionDescription.Type.OFFER) }
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `stop during suspended join disposes once and leaves late token without installing resources`() = runTest {
        val gate = VoiceResourceGate()
        val response = CompletableDeferred<String>()
        val left = mutableListOf<String>()
        val created = mutableListOf<String>()
        var token: String? = null
        val join = launch {
            val accepted = withContext(NonCancellable) {
                val received = response.await()
                if (gate.acceptToken(received) { token = it }) true
                else { left += received; false }
            }
            if (accepted) gate.use { created += "microphone" }
        }
        runCurrent() // Join has actually suspended before the synchronous stop.
        gate.use { created += "factory" }
        gate.close { created += "disposed" }
        gate.close { created += "double-disposal" }
        join.cancel()
        response.complete("late-token")
        join.join()
        assertEquals(listOf("factory", "disposed"), created)
        assertEquals(listOf("late-token"), left)
        assertNull(token)
        assertFalse(gate.isOpen)
    }

    @Test fun `JNI entry and teardown exclude each other without reopening a stopped microphone`() {
        val gate = VoiceResourceGate()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val events = mutableListOf<String>()
        val setup = Thread {
            gate.use {
                entered.countDown()
                check(release.await(2, TimeUnit.SECONDS))
                events += "constructed"
            }
        }
        val stop = Thread { gate.close { events += "disposed" } }
        setup.start()
        assertTrue(entered.await(2, TimeUnit.SECONDS))
        stop.start()
        release.countDown()
        setup.join(2000); stop.join(2000)
        assertFalse(setup.isAlive); assertFalse(stop.isAlive)
        assertEquals(listOf("constructed", "disposed"), events)
        assertThrows(IllegalStateException::class.java) { gate.use { events += "re-enabled" } }
    }

    @Test fun `pending ICE offer and ACK survive canceled recovery without sequence or SDP drift`() = runTest {
        val ledger = IceRestartLedger()
        val original = ledger.stage("turn-generation-A", "offer-A")
        val stalled = CompletableDeferred<Unit>()
        val recovery = launch { stalled.await(); ledger.answerApplied(original) }
        recovery.cancelAndJoin()
        assertSame(original, ledger.pending)
        assertEquals(original.offer(), ledger.pending!!.offer())
        assertEquals("offer-A", ledger.pending!!.sdp)
        ledger.recordAnswer(original, "answer-A")
        val canceledApply = CompletableDeferred<Unit>()
        val applying = launch { canceledApply.await(); ledger.answerApplied(original) }
        applying.cancelAndJoin()
        assertEquals("answer-A", ledger.pending!!.answerSdp)
        assertFalse(ledger.pending!!.answerApplied)
        ledger.answerApplied(original)
        val ack = ledger.pending!!.ack()
        assertEquals(original.ack(), ack)
        ledger.acknowledged(original)
        val next = ledger.stage("turn-generation-B", "offer-B")
        assertEquals(2L, next.sequence)
        assertNotEquals(original.offer(), next.offer())
    }

    @Test fun `retry policy distinguishes provider 403 from membership denial and malformed answers`() {
        assertTrue(retryableIceRestart(ApiException(403, "provider auth", "ice_restart_retry")))
        assertTrue(retryableIceRestart(ApiException(502, "temporary", "ice_restart_retry")))
        assertTrue(retryableIceRestart(ApiException(409, "busy", "ice_restart_pending")))
        assertTrue(retryableIceRestart(IOException("transport reset")))
        assertFalse(retryableIceRestart(ApiException(403, "membership denied")))
        assertFalse(retryableIceRestart(ApiException(401, "invalid capability")))
        assertFalse(retryableIceRestart(ApiException(502, "invalid answer", "ice_restart_invalid")))
        assertFalse(retryableIceRestart(IllegalStateException("invalid SDP")))
    }

    @Test fun `missing offer on immediate subscription negotiation is terminal`() {
        assertTrue(departedTrack(ApiException(404, "departed", "track_gone")))
        assertFalse(departedTrack(ApiException(404, "missing channel")))
        assertFalse(departedTrack(ApiException(403, "membership denied")))
        val missing = SignalResponse(tracks = listOf(SignalTrack("mid-A")), requiresImmediateRenegotiation = true)
        assertThrows(IllegalStateException::class.java) { subscriptionMid(missing) }
        val stable = SignalResponse(tracks = listOf(SignalTrack("mid-B")), requiresImmediateRenegotiation = false)
        assertEquals("mid-B", subscriptionMid(stable))
        val renegotiate = missing.copy(sessionDescription = SessionDescription("offer", "sdp"))
        assertEquals("mid-A", subscriptionMid(renegotiate))
    }
}
