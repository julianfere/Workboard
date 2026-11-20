// Coloreado de SQL (T-SQL) para el visor y los bloques ```sql del Markdown, como la v1: comentarios,
// cadenas, palabras clave y números. Todo el texto se escapa.

import { esc } from "./dom";

const SQL_KW =
  /\b(SELECT|FROM|WHERE|JOIN|LEFT|RIGHT|INNER|OUTER|CROSS|APPLY|ON|AND|OR|NOT|IN|EXISTS|INSERT|INTO|VALUES|UPDATE|SET|DELETE|MERGE|USING|WHEN|MATCHED|THEN|ELSE|END|CASE|BEGIN|COMMIT|ROLLBACK|TRAN|TRANSACTION|TRY|CATCH|CREATE|ALTER|DROP|TABLE|VIEW|PROCEDURE|PROC|FUNCTION|INDEX|TRIGGER|AS|IS|NULL|DECLARE|IF|WHILE|GO|GROUP|BY|ORDER|HAVING|TOP|DISTINCT|UNION|ALL|WITH|NOLOCK|OUTPUT|RAISERROR|THROW|PRINT|RETURN|TRUNCATE|EXEC|EXECUTE|CAST|CONVERT|COUNT|SUM|MAX|MIN|OVER|PARTITION|ROW_NUMBER|LIKE|BETWEEN|ASC|DESC|PRIMARY|KEY|FOREIGN|REFERENCES|CONSTRAINT|DEFAULT|IDENTITY|NVARCHAR|VARCHAR|INT|BIGINT|BIT|DATETIME2?|DATE|UNIQUEIDENTIFIER|DECIMAL|OBJECT_ID|COALESCE|ISNULL|XACT_STATE|XACT_ABORT|NOCOUNT|USE)\b/gi;

/** HTML del SQL coloreado (para meter en un `<pre>`). */
export function highlightSql(src: string): string {
  return src
    .split(/(--[^\n]*|\/\*[\s\S]*?\*\/|N?'(?:[^']|'')*')/)
    .map((part, i) => {
      if (i % 2) return /^(--|\/\*)/.test(part) ? `<span class="hl-cmt">${esc(part)}</span>` : `<span class="hl-str">${esc(part)}</span>`;
      // `esc` escapa con entidades numéricas (`&#39;`): esos dígitos no son números del SQL.
      return esc(part)
        .replace(SQL_KW, (w) => `<span class="hl-kw">${w}</span>`)
        .replace(/(?<!&#)\b(\d+(?:\.\d+)?)\b/g, '<span class="hl-num">$1</span>');
    })
    .join("");
}
