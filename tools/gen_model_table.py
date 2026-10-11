#!/usr/bin/env python3
"""Builds assets/models.json, the per-model command table AVR Control uses to
know exactly which commands and values a receiver supports.

Input is the YAML produced by import_protocol_doc_py3.py from Onkyo's
ISCP_AVR_*.xlsx protocol spreadsheet:

    python import_protocol_doc_py3.py ISCP_AVR_146.xlsx > eiscp-commands-146.yaml
    python tools/gen_model_table.py eiscp-commands-146.yaml > assets/models.json

Output format (models that share an identical command table share an entry):

    {"source": "...", "tables": [{"PWR": ["00", "01", "QSTN"], ...}, ...],
     "models": {"DTR-50.7": 12, ...}}

Ranges such as volume 0-100 are written as "0..100" (decimal).
"""
import json
import re
import sys

import yaml


class Loader(yaml.SafeLoader):
    pass


Loader.add_constructor("tag:yaml.org,2002:seq", lambda l, n: tuple(l.construct_sequence(n)))


def normalise_model(name):
    # "TX-NR646(Ether)" / "/515AE(Ether)" / "RDC-7(Ver2.0)" -> plain model names
    name = re.sub(r"\((Ether|Ver[\d.]+)\)", "", name).strip().lstrip("/")
    return name


def value_key(v):
    if isinstance(v, tuple):
        return "%d..%d" % (v[0], v[-1])
    return str(v)


def main(path):
    data = yaml.load(open(path), Loader=Loader)
    sets = data.pop("modelsets")
    per_model = {}
    for zone, commands in data.items():
        for cmd, info in commands.items():
            if not re.fullmatch(r"[A-Z0-9]{3}", str(cmd)):
                continue
            for value, vinfo in (info.get("values") or {}).items():
                for model in sets.get(vinfo.get("models"), ()):
                    table = per_model.setdefault(normalise_model(model), {})
                    values = table.setdefault(cmd, [])
                    key = value_key(value)
                    if key not in values:
                        values.append(key)

    tables, index, models = [], {}, {}
    for model in sorted(per_model):
        table = {k: per_model[model][k] for k in sorted(per_model[model])}
        blob = json.dumps(table, sort_keys=True)
        if blob not in index:
            index[blob] = len(tables)
            tables.append(table)
        models[model] = index[blob]
    out = {"source": path.rsplit("/", 1)[-1], "tables": tables, "models": models}
    json.dump(out, sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main(sys.argv[1])
