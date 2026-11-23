// Official public YouTube video. Use only its eleven-character video ID.
// The player loads after Play; there is no locally hosted video fallback.
export const video = Object.freeze({
  youtubeId: "4Vu0HkIB1gY",
  poster: "./assets/youtube-poster.jpg",
});

// Google Analytics 4 measurement ID (Admin → Data streams → Web, "G-XXXXXXXXXX").
// Leave empty to publish the site without analytics.
export const analytics = Object.freeze({
  measurementId: "G-N600K81XNJ",
});
