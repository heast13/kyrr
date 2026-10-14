# Security

kyrr installs an audio filter that runs inside the Windows audio engine and,
like Equalizer APO, switches on `DisableProtectedAudioDG` so Windows loads it.
On Linux it needs no extra rights: it runs as your user and adds a PipeWire
microphone.
Please report anything that could let other programs or users abuse that
privately through GitHub's
[security advisories](https://github.com/heast13/kyrr/security/advisories/new),
not as a public issue.
