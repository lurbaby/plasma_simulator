#!/usr/bin/env bash
# Запуск симулятора fusion-sim з веб-інтерфейсом і всіма моделями.
#   ./run_sim.sh            — зупинити старий сервер, запустити новий, відкрити браузер
#   ./run_sim.sh --build    — те саме, але спершу перезібрати (після змін у коді)
#   ./run_sim.sh --stop     — лише зупинити сервер
set -euo pipefail

SIM="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # тека репозиторію fusion-sim
ROOT="$(dirname "$SIM")"                                # поруч лежать models/ і fusion_work/
BIN="$SIM/target/release/fusionsim"
MODELS="$ROOT/models"
PY="$ROOT/fusion_work/venv/bin/python"   # numpy + sklearn: підходить для всіх моделей
PORT=9999
URL="http://127.0.0.1:$PORT/"
LOG="$SIM/target/serve.log"

stop_server() {
    if pgrep -x fusionsim >/dev/null; then
        echo "Зупиняю старий сервер…"
        pkill -x fusionsim || true
        for _ in $(seq 20); do pgrep -x fusionsim >/dev/null || break; sleep 0.25; done
    fi
}

case "${1:-}" in
    --stop) stop_server; echo "Сервер зупинено."; exit 0 ;;
    --build)
        echo "Збираю (WASM + fusionsim + web)…"
        export PATH="$HOME/.cargo/bin:$PATH"
        (cd "$SIM" && ./build.sh)
        ;;
    "") ;;
    *) echo "Невідомий ключ: $1 (є --build, --stop)"; exit 1 ;;
esac

# Перевірки
[[ -x "$BIN" ]]   || { echo "Немає $BIN — запусти: $0 --build"; exit 1; }
[[ -x "$PY" ]]    || { echo "Немає Python: $PY"; exit 1; }
[[ -d "$MODELS" ]] || { echo "Немає теки моделей: $MODELS"; exit 1; }
"$PY" -c "import numpy, sklearn" 2>/dev/null || { echo "У $PY бракує numpy/sklearn"; exit 1; }

stop_server

echo "Запускаю сервер (лог: $LOG)…"
cd "$SIM"
OMP_NUM_THREADS=1 nohup "$BIN" serve --models-dir "$MODELS" --python "$PY" >"$LOG" 2>&1 &
PID=$!

# Чекаємо, поки сервер відповість
for _ in $(seq 40); do
    if curl -s -o /dev/null "$URL"; then break; fi
    kill -0 "$PID" 2>/dev/null || { echo "Сервер впав:"; cat "$LOG"; exit 1; }
    sleep 0.25
done
curl -s -o /dev/null "$URL" || { echo "Сервер не відповідає за 10 с, див. $LOG"; exit 1; }

echo "Готово: $URL  (PID $PID)"
echo "  Демонстрація fixer: 🎛 Керування → модель v8_fixer → «Під'єднати» → Start"
echo "  Зупинити: $0 --stop"
command -v xdg-open >/dev/null && xdg-open "$URL" >/dev/null 2>&1 || true
