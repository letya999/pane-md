
"""Write the reproducible five-column Markdown fixture using only Python's standard library."""
import argparse
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("folder", type=Path)
parser.add_argument("--sections", type=int, default=160)
args = parser.parse_args()
root = args.folder
docs = root / "docs"
docs.mkdir(parents=True, exist_ok=True)
(docs / "Справочник.md").write_text("# Справочник\n\n## Методика\n\nЗдесь описана методика расчёта.\n", encoding="utf-8", newline="\n")

sections = ["---\nowner: review\n---\n\n# Время поставки\n\n"
            "Отчёт сравнивает время поставки по командам и помогает проверить исходные данные. "
            "Откройте [методику](docs/Справочник.md#методика) для подробностей.\n\n"
            "Найдите **P85** в таблице и добавьте комментарий. Контроль Unicode: Ж😀.\n\n"]
for number in range(1, args.sections + 1):
    sections.append(f"## Команда {number}\n\n"
                    "Период включает завершённые задачи. **Медиана** показывает типичное значение, "
                    "а _перцентили_ помогают увидеть задержки в длинном хвосте распределения.\n\n"
                    "| Показатель | Значение | Комментарий к расчёту | Методика | Проверка |\n"
                    "| :--- | ---: | :--- | :--- | :--- |\n"
                    f"| P85 | {number + 5} дней | Длинный текст в ячейке переносится на несколько строк "
                    "и остаётся выделяемым. Здесь приведены повторные наблюдения за завершёнными задачами. "
                    "| [Методика](docs/Справочник.md#методика) | Ж😀 |\n"
                    "| Медиана | 4 дня | Расчёт по рабочим дням с исключением остановок | см. выше | ОК |\n\n"
                    "- [x] Сверка источников\n- [ ] Проверка отчёта\n\n")
sections.append("## Приложение\n\nУточнение[^note].\n\n[^note]: Синтетические данные.\n\n"
                "```md\n<!-- pane-md:review {\"v\":1,\"id\":\"example\"} -->\n```\n\n<!-- обычный комментарий -->\n")
path = root / "Большой документ.md"
path.write_text("".join(sections), encoding="utf-8", newline="\n")
print(f"{path.name}: {path.stat().st_size} bytes, {args.sections} tables")
