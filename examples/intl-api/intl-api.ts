// Example: Intl API - Internationalization
// Demonstrates date, number, and collation formatting for different locales

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Format date according to locale
  function formatDate(dateStr: string, locale: string): Record<string, unknown> {
    const date = new Date(dateStr);
    const formatter = new Intl.DateTimeFormat(locale, {
      weekday: "long",
      year: "numeric",
      month: "long",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      timeZone: "UTC",
    });

    return {
      locale,
      date: dateStr,
      formatted: formatter.format(date),
      parts: formatter.formatToParts(date),
    };
  }

  // Format number according to locale
  function formatNumber(number: number, locale: string, style: string = "decimal"): Record<string, unknown> {
    const options: any = { style };

    if (style === "currency") {
      options.currency = locale === "en-US" ? "USD" : locale === "pt-BR" ? "BRL" : "EUR";
    } else if (style === "percent") {
      options.minimumFractionDigits = 2;
    }

    const formatter = new Intl.NumberFormat(locale, options);

    return {
      locale,
      number,
      style,
      formatted: formatter.format(number),
      parts: formatter.formatToParts(number),
    };
  }

  // Compare strings with locale-aware collation
  function compareStrings(locale: string, strings: string[]): Record<string, unknown> {
    const collator = new Intl.Collator(locale);
    const sorted = [...strings].sort(collator.compare);

    return {
      locale,
      original: strings,
      sorted,
    };
  }

  // Format relative time
  function formatRelativeTime(
    value: number,
    unit: Intl.RelativeTimeFormatUnit,
    locale: string
  ): Record<string, unknown> {
    const formatter = new Intl.RelativeTimeFormat(locale, {
      numeric: "auto",
    });

    return {
      locale,
      value,
      unit,
      formatted: formatter.format(value, unit),
    };
  }

  // Format by parts endpoint
  if (url.pathname === "/api/date" && req.method === "POST") {
    try {
      const { date, locale } = await req.json();
      return new Response(JSON.stringify(formatDate(date, locale), null, 2), {
        headers: { "content-type": "application/json" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({ error: "Date formatting failed" }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Number format endpoint
  if (url.pathname === "/api/number" && req.method === "POST") {
    try {
      const { number, locale, style } = await req.json();
      return new Response(JSON.stringify(formatNumber(number, locale, style), null, 2), {
        headers: { "content-type": "application/json" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({ error: "Number formatting failed" }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // String comparison endpoint
  if (url.pathname === "/api/collate" && req.method === "POST") {
    try {
      const { locale, strings } = await req.json();
      return new Response(JSON.stringify(compareStrings(locale, strings), null, 2), {
        headers: { "content-type": "application/json" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({ error: "Collation failed" }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Relative time endpoint
  if (url.pathname === "/api/reltime" && req.method === "POST") {
    try {
      const { value, unit, locale } = await req.json();
      return new Response(
        JSON.stringify(
          formatRelativeTime(value, unit, locale),
          null,
          2
        ),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({ error: "Relative time formatting failed" }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Intl API</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 1000px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          h2 { color: #667eea; margin: 30px 0 15px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          input, select, textarea { width: 100%; padding: 10px; margin: 5px 0; border: 1px solid #ddd; border-radius: 4px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin-top: 15px; max-height: 300px; overflow-y: auto; white-space: pre-wrap; font-size: 0.85em; }
          .locale-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin: 10px 0; }
          .locale-btn { padding: 8px; background: #e8f4f8; border: 1px solid #4a90e2; color: #4a90e2; cursor: pointer; border-radius: 4px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🌍 Intl API - Internationalization</h1>
          <p style="color: #999; margin-bottom: 20px;">Format dates, numbers, and text for different locales</p>

          <div class="section">
            <h2>1. Date Formatting</h2>
            <input type="datetime-local" id="dateInput" value="${new Date().toISOString().slice(0, 16)}">
            <select id="dateLocale">
              <option value="en-US">English (US)</option>
              <option value="pt-BR">Português (Brasil)</option>
              <option value="de-DE">Deutsch (Deutschland)</option>
              <option value="fr-FR">Français (France)</option>
              <option value="ja-JP">日本語</option>
              <option value="zh-CN">中文 (简体)</option>
              <option value="es-ES">Español</option>
            </select>
            <button onclick="formatDate()">Format Date</button>
            <div id="dateOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>2. Number Formatting</h2>
            <input type="number" id="numberInput" value="1234567.89" step="0.01">
            <select id="numberLocale">
              <option value="en-US">English (US)</option>
              <option value="pt-BR">Português (Brasil)</option>
              <option value="de-DE">Deutsch</option>
              <option value="fr-FR">Français</option>
            </select>
            <select id="numberStyle">
              <option value="decimal">Decimal</option>
              <option value="percent">Percentage</option>
              <option value="currency">Currency</option>
            </select>
            <button onclick="formatNumber()">Format Number</button>
            <div id="numberOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>3. String Collation (Sorting)</h2>
            <textarea id="stringInput" placeholder="Enter words (one per line)..." rows="5">zebra
apple
Äpfel
äpple
Banana</textarea>
            <select id="collateLocale">
              <option value="en-US">English (US)</option>
              <option value="de-DE">Deutsch</option>
              <option value="sv-SE">Svenska</option>
            </select>
            <button onclick="collateStrings()">Sort</button>
            <div id="collateOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>4. Relative Time</h2>
            <input type="number" id="relTimeValue" value="-3">
            <select id="relTimeUnit">
              <option value="year">Year(s)</option>
              <option value="month">Month(s)</option>
              <option value="week">Week(s)</option>
              <option value="day">Day(s)</option>
              <option value="hour">Hour(s)</option>
              <option value="minute">Minute(s)</option>
              <option value="second">Second(s)</option>
            </select>
            <select id="relTimeLocale">
              <option value="en-US">English (US)</option>
              <option value="pt-BR">Português (Brasil)</option>
              <option value="ja-JP">日本語</option>
            </select>
            <button onclick="formatRelTime()">Format</button>
            <div id="relTimeOutput" class="output"></div>
          </div>
        </div>

        <script>
          async function formatDate() {
            const date = document.getElementById('dateInput').value;
            const locale = document.getElementById('dateLocale').value;
            try {
              const response = await fetch('/api/date', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ date: new Date(date).toISOString(), locale })
              });
              const data = await response.json();
              document.getElementById('dateOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('dateOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function formatNumber() {
            const number = parseFloat(document.getElementById('numberInput').value);
            const locale = document.getElementById('numberLocale').value;
            const style = document.getElementById('numberStyle').value;
            try {
              const response = await fetch('/api/number', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ number, locale, style })
              });
              const data = await response.json();
              document.getElementById('numberOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('numberOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function collateStrings() {
            const strings = document.getElementById('stringInput').value.split('\\n').filter(s => s.trim());
            const locale = document.getElementById('collateLocale').value;
            try {
              const response = await fetch('/api/collate', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ locale, strings })
              });
              const data = await response.json();
              document.getElementById('collateOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('collateOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function formatRelTime() {
            const value = parseInt(document.getElementById('relTimeValue').value);
            const unit = document.getElementById('relTimeUnit').value;
            const locale = document.getElementById('relTimeLocale').value;
            try {
              const response = await fetch('/api/reltime', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ value, unit, locale })
              });
              const data = await response.json();
              document.getElementById('relTimeOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('relTimeOutput').textContent = 'Error: ' + e.message;
            }
          }
        </script>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  return new Response("Not found", { status: 404 });
});
