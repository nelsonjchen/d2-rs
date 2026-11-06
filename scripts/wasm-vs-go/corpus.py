#!/usr/bin/env python3
"""WASM jobs using the shared CLI differential corpus and mutations."""
import argparse, itertools, json, random, importlib.util, sys
from pathlib import Path
SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("differential_scoreboard", SCRIPTS / "differential-scoreboard.py")
differential = importlib.util.module_from_spec(spec)
spec.loader.exec_module(differential)
inputs, mutate = differential.inputs, differential.mutate

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--seed", type=int, default=20261102)
    p.add_argument("--cap", type=int, default=2000)
    args = p.parse_args()
    rng = random.Random(args.seed)
    corpus = list(inputs())
    rng.shuffle(corpus)
    jobs = []
    option_rng = random.Random(args.seed ^ 0xD2)
    # Continue drawing mutations after the first pass so a larger cap really
    # widens coverage. The first pass retains its historical order and RNG.
    for index, (name, source) in enumerate(itertools.cycle(corpus)):
        mutated, kind = mutate(source, rng)
        variants = ((source, "original"), (mutated, kind)) if index < len(corpus) else ((mutated, kind),)
        for text, mutation in variants:
            i = len(jobs)
            variant = (i // 2) % 24
            options = {"layout": ("dagre", "elk", "tala")[variant % 3],
                       "themeID": (0, 200)[(variant // 3) % 2],
                       "sketch": bool((variant // 6) % 2),
                       "pad": (0, 20)[(variant // 12) % 2]}
            if mutation == "layer": options["target"] = "layers.differential_layer"
            for probability, key, value in ((12, "darkThemeID", 200), (10, "center", True),
                    (12, "scale", 0.5), (16, "forceAppendix", True), (12, "noXMLTag", True),
                    (12, "salt", "differential"), (100, "isometric", True), (30, "ascii", True)):
                if option_rng.randrange(probability) == 0: options[key] = value
            if options.get("ascii"): options["asciiMode"] = option_rng.choice(("standard", "extended"))
            if option_rng.randrange(4) == 0: options["themeID"] = option_rng.choice((1, 3, 4, 5, 100, 101, 300, 301))
            jobs.append(dict(name=name, source=text, mutation=mutation, options=options))
            if len(jobs) >= args.cap: break
        if len(jobs) >= args.cap: break
    # Keep the minimized image-node response witness in every run without
    # perturbing the sampled inputs or their option matrix.
    for engine in ("dagre", "elk", "tala"):
        jobs.append(dict(name="fixed:image-icon-position", mutation="original",
                         source="a: {shape: image; icon: https://example.com/a.svg}",
                         options={"layout": engine, "themeID": 0, "sketch": False, "pad": 20}))
    jobs.append(dict(name="fixed:quoted-label-ast", mutation="original",
                     source="a: 'it''s quoted'\na -> b: '11'\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": False, "pad": 20}))
    jobs.append(dict(name="fixed:tala-sequence-edges", mutation="original",
                     source="a.shape: step\nb.shape: step\nc.shape: step\na -- b -- c\na -> d: 1\n",
                     options={"layout": "tala", "themeID": 0, "sketch": False, "pad": 20}))
    jobs.append(dict(name="fixed:quoted-key-references", mutation="original",
                     source="'Data' -- y\nx -- 'Data'\n'Data': {style.fill: red}\n",
                     options={"layout": "tala", "themeID": 0, "sketch": True, "pad": 0}))
    jobs.append(dict(name="fixed:tala-grid-font", mutation="original",
                     source='grid: "" {grid-rows: 2; a; b}\nempty: {grid-rows: 2}\nstyled: {grid-rows: 2; style.font-size: 20; a; b}\n',
                     options={"layout": "tala", "themeID": 0, "sketch": True, "pad": 20}))
    jobs.append(dict(name="fixed:sequence-class-position", mutation="original",
                     source="shape: sequence_diagram\na: {shape: class; style.font-color: red}\n",
                     options={"layout": "dagre", "themeID": 200, "sketch": False, "pad": 20}))
    jobs.append(dict(name="fixed:near-image-language", mutation="original",
                     source="description: |md\n# Hello\n|\nimage: {shape: image; icon: https://example.com/a.svg; near: center-left; width: 200; height: 300}\na -> b\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": False, "pad": 0}))
    # Supplemental mutation kinds follow the historical sample and witnesses
    # so widening coverage never renumbers previously reported cases.
    additions = (
        ("code", 'differential_code: |go\npackage main\nfunc main() { println("雪") }\n|\n'),
        ("scenario", "scenarios: {differential_scenario: {a -> b}}\n"),
        ("step", "steps: {differential_step: {a -> b}}\n"),
    )
    for name, source in corpus[:min(12, max(1, args.cap // 100))]:
        for kind, addition in additions:
            variant = (len(jobs) // 2) % 24
            jobs.append(dict(name=name, source=source + "\n" + addition, mutation=kind,
                             options={"layout": ("dagre", "elk", "tala")[variant % 3],
                                      "themeID": (0, 200)[(variant // 3) % 2],
                                      "sketch": bool((variant // 6) % 2),
                                      "pad": (0, 20)[(variant // 12) % 2]}))
    jobs.append(dict(name="fixed:resolved-loop", mutation="original",
                     source="a: { b; c }; a.c -- a",
                     options={"layout": "dagre", "themeID": 0, "sketch": False, "pad": 0}))
    jobs.append(dict(name="fixed:grid-growth-rounding", mutation="original",
                     source="grid-rows: 2; a.width: 820; b.width: 200; c.width: 150; d.width: 100; e.width: 50\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": False, "pad": 0}))
    jobs.append(dict(name="fixed:nested-grid-growth-rounding", mutation="original",
                     source="g: {grid-rows: 2; a.width: 820; b.width: 200; c.width: 150; d.width: 100; e.width: 50}\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": False, "pad": 0}))
    jobs.append(dict(name="fixed:tala-mirror-transaction", mutation="original",
                     source="""non container: {
  Default.class: icon
  OutsideTopLeft.class: [icon; OutsideTopLeft]
  OutsideTopCenter.class: [icon; OutsideTopCenter]
  InsideBottomLeft.class: [icon; InsideBottomLeft]
  InsideBottomCenter.class: [icon; InsideBottomCenter]
  InsideBottomRight.class: [icon; InsideBottomRight]
}
container: {
  InsideBottomRight.InsideBottomRight
}
image: {
  OutsideLeftMiddle.class: [icon; image; OutsideLeftMiddle]
  OutsideLeftBottom.class: [icon; image; OutsideLeftBottom]
  OutsideRightTop.class: [icon; image; OutsideRightTop]
  InsideMiddleLeft.class: [icon; image; InsideMiddleLeft]
  InsideMiddleCenter.class: [icon; image; InsideMiddleCenter]
  InsideMiddleRight.class: [icon; image; InsideMiddleRight]
  InsideBottomLeft.class: [icon; image; InsideBottomLeft]
  InsideBottomCenter.class: [icon; image; InsideBottomCenter]
  InsideBottomRight.class: [icon; image; InsideBottomRight]
}
non container -> container -> image
classes: {
  image.shape: image
  icon.icon: https://icons.terrastruct.com/essentials/time.svg
  OutsideTopLeft.icon.near: outside-top-left
  OutsideTopCenter.icon.near: outside-top-center
  OutsideLeftMiddle.icon.near: outside-left-center
}
""",
                     options={"layout": "tala", "themeID": 200, "sketch": True, "pad": 0}))
    for name, source, _, flags in differential.fixed_jobs():
        if name.startswith("fixed:review-"):
            jobs.append(dict(name=name, source=source, mutation="original", options={"layout": "dagre"}))
    for options in ({"darkThemeID": 200}, {"scale": 0.5}, {"center": True},
                    {"ascii": True, "asciiMode": "standard"}, {"isometric": True},
                    {"target": "layers.differential_layer"}):
        jobs.append(dict(name="fixed:options-" + next(iter(options)), mutation="original",
                         source="a -> b\nlayers: {differential_layer: {c -> d}}\n", options=options))
    jobs.append(dict(name="fixed:tala-transpose-tree-obstructions", mutation="original",
                     source="a -> d\na -> e\na -> f\ng -> a\na -> h\nd -> p\ne -> r\np -> s\nf -> t\nf -> u\nw -> h\ndifferential_a -> differential_b\n",
                     options={"layout": "tala", "themeID": 200, "sketch": False, "pad": 0}))
    jobs.append(dict(name="fixed:sequence-late-group-actor", mutation="original",
                     source="shape: sequence_diagram\na\ng: {h: {a -> b}}\na -> h\n",
                     options={"layout": "tala"}))
    jobs.append(dict(name="fixed:elk-fractional-boundary-port", mutation="original",
                     source="network: 通信網 {\n  cell tower: {\n\t\tsatellites: 衛星 {\n\t\t}\n\t\tsatellites -> transmitter: SEnD {\n\t\t}\n  }\n  online portal: オンラインポータル {\n  }\n  data processor: データプロセッサ {\n    storage: 保管所 {\n    }\n  }\n}\nuser -> network.cell tower: 電話をかける\nuser -> network.online portal.ui: アクセス {\n}\napi server: API サーバー {\n}\napi server -> network.online portal.ui: 画面\napi server -> logs: 持続する\nlogs: ログ { shape: page; style.multiple: true }\n\nnetwork.data processor -> api server\n",
                     options={"layout": "elk", "themeID": 200, "sketch": False, "pad": 20}))
    jobs.append(dict(name="fixed:dagre-parallel-compound-outside-node", mutation="original",
                     source="defendants: {\n  chesscom -> naka: Sponsoring\n  chesscom -> naka: Sponsoring\n}\nhans -> defendants: 'sueing for $100M'\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": True, "pad": 20,
                              "scale": 0.5, "forceAppendix": True}))
    jobs.append(dict(name="fixed:dagre-authored-container-fit", mutation="original",
                     source="a: {\n  width: 500\n\te: {\n\t}\n}\n",
                     options={"layout": "dagre", "themeID": 200, "sketch": False,
                              "pad": 20, "darkThemeID": 200}))
    jobs.append(dict(name="fixed:class-image-without-local-shape", mutation="original",
                     source="classes: {i: {shape: image; width: 70; height: 70}}\na: {class: i}\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": True, "pad": 0}))
    jobs.append(dict(name="fixed:dagre-small-seen-map-growth", mutation="original",
                     source="user01: User01 {\n  style: {\n    font-size: 28\n  }\n}\ncontainer_producer_01: Service01 {\n  task01: {\n    producer: Producer {\n      style: {\n        font: mono\n      }\n    }\n    style: {\n      font-size: 28\n    }\n  }\n  task02 -> queue.producer: Enqueue Task {\n  }\n}\ncontainer_producer_02: Service02 {\n  style: {\n    font-size: 28\n  }\n  task01: {\n    style: {\n      font-size: 32\n    }\n  }\n  task02: {\n    producer: Producer {\n      style: {\n        font: mono\n      }\n    }\n  }\n  task01 -> queue.producer: Enqueue Task {\n  }\n}\ndatabase: Ring\\nBuffer {\n  style: {\n    bold: true\n  }\n}\nother_store: \"\" {\n  style: {\n    opacity: 0\n  }\n  redis: Redis {\n    style: {\n      bold: true\n    }\n  }\n  rabbitmq: RabbotMQ {\n    style: {\n      font: mono\n    }\n  }\n  nats: NATS {\n    style: {\n      opacity: 0.7\n    }\n  }\n}\ncontainer_consumer_01: Worker01 {\n  task01: {\n    style: {\n      multiple: true\n      font-size: 32\n    }\n  }\n  queue: Queue Library {\n    consumer: Consumer {\n      style: {\n        bold: true\n        font-size: 32\n        font: mono\n      }\n    }\n    style: {\n      animated: true\n      bold: true\n      font-size: 28\n    }\n  }\n  queue.consumer -> task02: Handle Task {\n    style: {\n      bold: true\n      font-size: 28\n    }\n  }\n}\ncontainer_consumer_02: Worker02 {\n  direction: right\n  style: {\n    bold: true\n    font-size: 28\n  }\n  queue01.consumer -> task01: Handle Task {\n    style: {\n      animated: true\n      bold: true\n      font-size: 28\n    }\n  }\n  queue01.consumer -> task02: Handle Task {\n    style: {\n      bold: true\n      font-size: 28\n    }\n    style: {\n      animated: true\n      bold: true\n      font-size: 28\n    }\n  }\n}\nuser01 -> container_producer_01.task01: Create Task {\n}\nuser02 -> container_producer_01.task02: Create Task {\n  style: {\n    animated: true\n    bold: true\n    font-size: 28\n  }\n}\nuser03 -> container_producer_02.task01: Create Task {\n  style: {\n    animated: true\n    bold: true\n  }\n}\ncontainer_producer_01.queue.producer -> database: Store Task {\n  style: {\n    animated: true\n    bold: true\n    font-size: 28\n  }\n}\ncontainer_producer_02.queue.producer -> other_store.rabbitmq: Store Task {\n  style: {\n    animated: true\n    bold: true\n    opacity: 0.3\n  }\n}\nother_store.nats -> container_consumer_02.queue02.consumer: \"\" {\n}\n",
                     options={"layout": "dagre", "themeID": 200, "sketch": False, "pad": 0}))
    for kind in ("scenarios", "steps"):
        jobs.append(dict(name="fixed:unformatted-child-ast-" + kind, mutation="original",
                         source="'key': 'it''s quoted'\na: {icon: 'https://example.com/a.svg'}\n" + kind + ": {one: {b}}\n",
                         options={"layout": "dagre"}))
    jobs.append(dict(name="fixed:bottom-center-near-arithmetic", mutation="original",
                     source="z: {near: bottom-center; a -> b; c -> d}\nc -> c\n",
                     options={"layout": "dagre", "themeID": 100, "sketch": False,
                              "pad": 20, "center": True}))
    for engine in ("dagre", "elk", "tala"):
        jobs.append(dict(name="fixed:d2-config-object", mutation="original",
                         source="a -> d2-config\nd2-config: {b -> c}\n",
                         options={"layout": engine}))
    jobs.append(dict(name="fixed:dagre-outside-label-cross-margin", mutation="original",
                     source="p: {a: {shape: image; icon: https://example.com/a.svg; label: label; label.near: outside-right-center}; b: {shape: image; icon: https://example.com/a.svg}}\n",
                     options={"layout": "dagre", "themeID": 200, "pad": 20}))
    jobs.append(dict(name="fixed:dagre-source-order-spacing", mutation="original",
                     source="direction: right\na.f.g -> s.n\nu.c -> s.z.c\na.f.i\nf -> s\n",
                     options={"layout": "dagre", "themeID": 301, "sketch": True,
                              "pad": 20}))
    jobs.append(dict(name="fixed:dagre-empty-sequence-near", mutation="original",
                     source="a.shape: sequence_diagram\nn.near: top-left\n",
                     options={"layout": "dagre", "themeID": 0, "sketch": False,
                              "pad": 20}))
    print(json.dumps(jobs, ensure_ascii=False))
