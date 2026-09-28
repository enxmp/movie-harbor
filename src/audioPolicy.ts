type AudioSource = {
  codec_name: string;
  profile?: string;
  channels?: number;
  channel_layout?: string;
  tags?: { title?: string; name?: string };
};
export const eac3Layouts = [
  "mono",
  "stereo",
  "3.0(back)",
  "3.0",
  "quad(side)",
  "quad",
  "4.0",
  "5.0(side)",
  "5.0",
  "2 channels (FC+LFE)",
  "2.1",
  "4 channels (FL+FR+LFE+BC)",
  "3.1",
  "4.1",
  "5.1(side)",
  "5.1",
];
export function canEac3(s: AudioSource) {
  return (
    !!s.channels &&
    s.channels <= 6 &&
    eac3Layouts.includes(s.channel_layout || "")
  );
}
export function recommendedAudio(s: AudioSource): string {
  const description = [s.profile, s.tags?.title, s.tags?.name].join(" ");
  if (
    !s.channels ||
    s.channels > 6 ||
    /atmos|dts[: -]?x/i.test(description) ||
    /dts-hd|master audio/i.test(s.profile || "")
  )
    return "copy";
  if (
    [
      "aac",
      "ac3",
      "eac3",
      "opus",
      "mp3",
      "vorbis",
      "flac",
      "alac",
      "truehd",
      "mlp",
      "wavpack",
      "ape",
    ].includes(s.codec_name) ||
    s.codec_name.startsWith("pcm_")
  )
    return "copy";
  if (s.channels <= 2) return "aac";
  return canEac3(s) ? "eac3" : "copy";
}
