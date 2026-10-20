# Measures word error rate of a kyrr dictation model with sherpa-onnx, the way
# kyrr's crates/kyrr-dictate loads it (nemo_transducer, feature_dim 128, 16 kHz).
#
# Test clips come from Hugging Face's datasets server: the first N of
# Multilingual LibriSpeech (test) for de, fr, es, it, nl, pl, pt, and the first
# 73 of LibriSpeech (validation, clean) for en. Text is compared lowercase and
# without punctuation. Clips are cached as 16 kHz wav in --cache (not in the
# repository).
#
# Usage:
#   wer.py --model <model dir> --cache <cache dir> [--langs fr,es,...]
#          [--clips 100] [--threads 4] [--short 10] [--out result.json]
#
# --short N additionally cuts the first ~3 s of N clips per language and
# reports how many came out in the right language (lingua, restricted to the
# tested languages) and in Latin script.
#
# Needs: pip install sherpa-onnx==1.13.8 soundfile scipy numpy psutil
# lingua-language-detector (only for --short). Each call measures one model, so
# the RAM figure (resident-set growth while loading) is not skewed by earlier runs.
import argparse, json, os, re, sys, time, urllib.request
import numpy as np
import psutil, soundfile as sf
from scipy.signal import resample_poly
import sherpa_onnx

API = "https://datasets-server.huggingface.co/rows"
RATE = 16000
SOURCES = {  # lang: (dataset, config, split, text field)
    "de": ("facebook/multilingual_librispeech", "german", "test", "transcript"),
    "fr": ("facebook/multilingual_librispeech", "french", "test", "transcript"),
    "es": ("facebook/multilingual_librispeech", "spanish", "test", "transcript"),
    "it": ("facebook/multilingual_librispeech", "italian", "test", "transcript"),
    "nl": ("facebook/multilingual_librispeech", "dutch", "test", "transcript"),
    "pl": ("facebook/multilingual_librispeech", "polish", "test", "transcript"),
    "pt": ("facebook/multilingual_librispeech", "portuguese", "test", "transcript"),
    "en": ("openslr/librispeech_asr", "clean", "validation", "text"),
}
EN_CLIPS = 73


def get(url, tries=6):
    for i in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "kyrr-wer"}), timeout=60) as r:
                return r.read()
        except Exception as e:
            if i == tries - 1:
                raise
            time.sleep(2 ** i)


def load_clips(lang, n, cache):
    """[(samples float32 16 kHz, reference text)] for the first n clips."""
    ds, cfg, split, field = SOURCES[lang]
    n = EN_CLIPS if lang == "en" else n
    d = os.path.join(cache, lang)
    os.makedirs(d, exist_ok=True)
    clips = []
    for off in range(0, n, 100):  # the rows API gives at most 100 per request
        length = min(100, n - off)
        meta = os.path.join(d, f"rows-{off}-{length}.json")
        if os.path.exists(meta) and all(os.path.exists(os.path.join(d, f"{off + i}.wav")) for i in range(length)):
            texts = json.load(open(meta, encoding="utf-8"))
        else:
            q = f"{API}?dataset={ds}&config={cfg}&split={split}&offset={off}&length={length}"
            rows = json.loads(get(q))["rows"]
            texts = []
            for i, r in enumerate(rows):
                texts.append(r["row"][field])
                raw = get(r["row"]["audio"][0]["src"])
                tmp = os.path.join(d, f"{off + i}.src")
                open(tmp, "wb").write(raw)
                x, sr = sf.read(tmp, dtype="float32")
                os.remove(tmp)
                if x.ndim > 1:
                    x = x.mean(axis=1)
                if sr != RATE:
                    x = resample_poly(x, RATE, sr).astype(np.float32)
                sf.write(os.path.join(d, f"{off + i}.wav"), x, RATE, subtype="FLOAT")
            json.dump(texts, open(meta, "w", encoding="utf-8"), ensure_ascii=False)
        for i, t in enumerate(texts):
            x, _ = sf.read(os.path.join(d, f"{off + i}.wav"), dtype="float32")
            clips.append((x, t))
    return clips


def norm(s):
    return re.sub(r"[^\w\s]|_", " ", s.lower()).split()


def edits(ref, hyp):
    prev = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        cur = [i]
        for j, h in enumerate(hyp, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (r != h)))
        prev = cur
    return prev[-1]


def make_recognizer(model, threads):
    return sherpa_onnx.OfflineRecognizer.from_transducer(
        encoder=f"{model}/encoder.int8.onnx", decoder=f"{model}/decoder.int8.onnx",
        joiner=f"{model}/joiner.int8.onnx", tokens=f"{model}/tokens.txt",
        num_threads=threads, sample_rate=RATE, feature_dim=128,
        model_type="nemo_transducer", decoding_method="greedy_search",
    )


def transcribe(rec, x):
    s = rec.create_stream()
    s.accept_waveform(RATE, x)
    rec.decode_stream(s)
    return s.result.text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--cache", required=True)
    ap.add_argument("--langs", default="de,en,fr,es,it,nl,pl,pt")
    ap.add_argument("--clips", type=int, default=100)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--short", type=int, default=0)
    ap.add_argument("--out")
    a = ap.parse_args()
    langs = a.langs.split(",")

    proc = psutil.Process()
    before = proc.memory_info().rss
    t0 = time.time()
    rec = make_recognizer(a.model, a.threads)
    load_s = time.time() - t0
    ram_mb = (proc.memory_info().rss - before) / 2**20
    result = {"model": os.path.basename(os.path.normpath(a.model)), "sherpa_onnx": sherpa_onnx.__version__,
              "load_seconds": round(load_s, 1), "ram_at_load_mb": round(ram_mb), "langs": {}}
    print(f"{result['model']}: loaded in {load_s:.1f} s, +{ram_mb:.0f} MB resident")

    detector = None
    if a.short:
        from lingua import Language, LanguageDetectorBuilder
        L = {"de": Language.GERMAN, "en": Language.ENGLISH, "fr": Language.FRENCH, "es": Language.SPANISH,
             "it": Language.ITALIAN, "nl": Language.DUTCH, "pl": Language.POLISH, "pt": Language.PORTUGUESE}
        detector = LanguageDetectorBuilder.from_languages(*L.values()).build()
        isocode = lambda t: next((k for k, v in L.items() if v == detector.detect_language_of(t)), None) if t.strip() else None

    for lang in langs:
        clips = load_clips(lang, a.clips, a.cache)
        words = errs = 0
        secs = dec = 0.0
        hyps = []
        for x, ref in clips:
            t = time.time()
            hyp = transcribe(rec, x)
            dec += time.time() - t
            secs += len(x) / RATE
            r, h = norm(ref), norm(hyp)
            words += len(r)
            errs += edits(r, h)
            hyps.append(hyp)
        entry = {"clips": len(clips), "words": words, "wer_percent": round(100 * errs / words, 2),
                 "audio_seconds": round(secs), "x_realtime": round(secs / dec, 1),
                 "first_hypotheses": [[c[1], h] for c, h in list(zip(clips, hyps))[:3]]}
        if a.short:
            ok = latin = 0
            cut = []
            for x, _ in clips[: a.short]:
                h = transcribe(rec, x[: 3 * RATE])
                cut.append(h)
                ok += isocode(h) == lang
                latin += bool(h.strip()) and all(c.isascii() or c.isalpha() and ord(c) < 0x250 for c in h)
            entry["short"] = {"clips": min(a.short, len(clips)), "right_language": ok, "latin_script": latin, "texts": cut}
        result["langs"][lang] = entry
        s = entry.get("short")
        print(f"  {lang}: WER {entry['wer_percent']:.2f} % ({len(clips)} clips, {words} words), "
              f"{entry['x_realtime']}x real time" + (f"; 3 s clips: {s['right_language']}/{s['clips']} right language, "
              f"{s['latin_script']}/{s['clips']} Latin script" if s else ""))
    if a.out:
        json.dump(result, open(a.out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
