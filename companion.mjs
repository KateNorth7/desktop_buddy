// The three cards the landing page's companion shows, each a card the app
// really sends: a completion offer, a deadline check-in and a new-work offer.
// The first choice is the primary one. Replies are shown for REPLY_MS.
export const REPLY_MS = 5000;

export const COMPANION_CARDS = Object.freeze([
  {
    id: "completion",
    before: "Mark ",
    goal: "Prepare the autumn release",
    after: " complete?",
    choices: [
      {
        id: "yes",
        label: "Yes, done",
        reply: "Closed Prepare the autumn release.",
        undo: true,
      },
      {
        id: "not_yet",
        label: "Not yet",
        reply: "Okay. I will ask again when you switch away.",
      },
    ],
  },
  {
    id: "deadline",
    before: "",
    goal: "Prepare the autumn release",
    after: " is due tomorrow at 10:00.",
    choices: [
      { id: "on_it", label: "I'm on it", reply: "Noted. No more reminders today." },
      { id: "reschedule", label: "Reschedule", reply: "Moved to Thursday, 10:00." },
    ],
  },
  {
    id: "new_work",
    before: "40 minutes in Figma on the release mockups. Add ",
    goal: "Review the mockups",
    after: " to today?",
    choices: [
      {
        id: "add",
        label: "Add",
        reply: "Added to today, under Prepare the autumn release.",
      },
      { id: "ignore", label: "Ignore", reply: "Ignored. I will not suggest it again." },
    ],
  },
]);
