import { createContext } from "react";

/** A send is pending; replies wait for it. A context rather than a row prop,
 * so sending re-renders the rows' Reply buttons and not the rows. */
export const SendPendingContext = createContext(false);
