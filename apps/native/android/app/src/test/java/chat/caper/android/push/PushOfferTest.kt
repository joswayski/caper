package chat.caper.android.push

import org.junit.Assert.*
import org.junit.Test

class PushOfferTest {
    private fun offer(firebase: Boolean = true, turnedOff: Boolean = false, offered: Boolean = true,
                      permitted: Boolean = false, asked: Boolean = false) =
        pushOffer(firebase, turnedOff, offered, permitted, asked)

    @Test fun opening_the_app_asks_once_and_then_respects_the_answer() {
        assertEquals(PushOffer.ASK, offer())
        assertEquals("A denied prompt is never shown again by app open", PushOffer.NONE, offer(asked = true))
        assertEquals("Allowing later in Android settings turns push on", PushOffer.ENABLE, offer(asked = true, permitted = true))
    }

    @Test fun permission_already_granted_turns_push_on_without_asking() {
        assertEquals(PushOffer.ENABLE, offer(permitted = true))
    }

    @Test fun turning_it_off_or_no_fcm_means_nothing_happens() {
        assertEquals(PushOffer.NONE, offer(turnedOff = true))
        assertEquals(PushOffer.NONE, offer(turnedOff = true, permitted = true))
        assertEquals(PushOffer.NONE, offer(offered = false))
        assertEquals(PushOffer.NONE, offer(firebase = false, permitted = true))
    }
}
