/** Independent local timing variation, generated once per demo loop. */
export function createDemoTiming(speech: number[][][], posts: number[], random = Math.random) {
  const jitter = (amount: number) => (random() * 2 - 1) * amount;
  return {
    speech: speech.map((windows) =>
      windows.map(([start, end]) => {
        const shifted = Math.max(0, start + jitter(0.45));
        return [shifted, shifted + end - start + jitter(Math.min(0.45, (end - start) * 0.2))];
      }),
    ),
    messages: posts.map((post, index) => {
      // Keep the three introductory messages stable through hydration. Small
      // later offsets preserve reply order, bursts and channel membership.
      const at = post + (index < 3 ? 0 : jitter(0.2));
      let reaction = at;
      const reactions = Array.from({ length: 4 }, (_, index) => {
        reaction += index === 0 ? 1.5 + random() * 1.5 : 0.8 + random() * 2.4;
        return reaction;
      });
      return { at, reactions, typing: at - (1.5 + random() * 4) };
    }),
  };
}
