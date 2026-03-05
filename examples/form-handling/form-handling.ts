// Example: Form Handling
// Demonstrates handling HTML forms (GET and POST)

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Display form
  if (url.pathname === "/" && req.method === "GET") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>Form Handling Example</title>
        <style>
          * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
          }

          body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
          }

          .container {
            background: white;
            border-radius: 8px;
            padding: 40px;
            box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
            max-width: 500px;
            width: 100%;
          }

          h1 {
            color: #333;
            margin-bottom: 10px;
            font-size: 2em;
          }

          .subtitle {
            color: #999;
            margin-bottom: 30px;
          }

          .form-group {
            margin-bottom: 20px;
          }

          label {
            display: block;
            color: #333;
            font-weight: 500;
            margin-bottom: 8px;
          }

          input[type="text"],
          input[type="email"],
          input[type="number"],
          input[type="date"],
          textarea,
          select {
            width: 100%;
            padding: 12px;
            border: 1px solid #ddd;
            border-radius: 4px;
            font-size: 1em;
            font-family: inherit;
            transition: border-color 0.3s;
          }

          input[type="text"]:focus,
          input[type="email"]:focus,
          input[type="number"]:focus,
          input[type="date"]:focus,
          textarea:focus,
          select:focus {
            outline: none;
            border-color: #667eea;
            box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
          }

          textarea {
            resize: vertical;
            min-height: 100px;
          }

          .checkbox-group {
            display: flex;
            align-items: center;
            gap: 10px;
          }

          input[type="checkbox"] {
            width: 18px;
            height: 18px;
            cursor: pointer;
          }

          .radio-group {
            display: flex;
            gap: 20px;
            margin-top: 8px;
          }

          input[type="radio"] {
            width: 18px;
            height: 18px;
            cursor: pointer;
          }

          button {
            width: 100%;
            padding: 12px;
            background: #667eea;
            color: white;
            border: none;
            border-radius: 4px;
            font-size: 1em;
            font-weight: 600;
            cursor: pointer;
            transition: background 0.3s;
          }

          button:hover {
            background: #764ba2;
          }

          button:active {
            transform: scale(0.98);
          }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>📝 Form Example</h1>
          <p class="subtitle">Submit the form to see the data processing</p>

          <form method="POST" action="/">
            <div class="form-group">
              <label for="name">Full Name *</label>
              <input type="text" id="name" name="name" required>
            </div>

            <div class="form-group">
              <label for="email">Email Address *</label>
              <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
              <label for="age">Age</label>
              <input type="number" id="age" name="age" min="0" max="150">
            </div>

            <div class="form-group">
              <label for="country">Country</label>
              <select id="country" name="country">
                <option value="">Select a country</option>
                <option value="us">United States</option>
                <option value="br">Brazil</option>
                <option value="uk">United Kingdom</option>
                <option value="de">Germany</option>
                <option value="other">Other</option>
              </select>
            </div>

            <div class="form-group">
              <label for="date">Date of Visit</label>
              <input type="date" id="date" name="date">
            </div>

            <div class="form-group">
              <label for="message">Message</label>
              <textarea id="message" name="message" placeholder="Enter your message..."></textarea>
            </div>

            <div class="form-group">
              <label>Newsletter</label>
              <div class="checkbox-group">
                <input type="checkbox" id="newsletter" name="newsletter">
                <label for="newsletter" style="margin: 0;">Subscribe to our newsletter</label>
              </div>
            </div>

            <div class="form-group">
              <label>Preference</label>
              <div class="radio-group">
                <div>
                  <input type="radio" id="option1" name="preference" value="option1">
                  <label for="option1" style="margin: 0;">Option 1</label>
                </div>
                <div>
                  <input type="radio" id="option2" name="preference" value="option2">
                  <label for="option2" style="margin: 0;">Option 2</label>
                </div>
              </div>
            </div>

            <button type="submit">Submit Form</button>
          </form>
        </div>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // Handle form submission
  if (url.pathname === "/" && req.method === "POST") {
    try {
      const formData = await req.formData();

      // Extract form values
      const data: Record<string, string | boolean | null> = {};
      for (const [key, value] of formData) {
        if (typeof value === "string") {
          data[key] = value;
        }
      }

      // Validate required fields
      if (!data.name || !data.email) {
        return new Response(
          JSON.stringify({
            error: "Name and email are required",
            received: data,
          }),
          {
            status: 400,
            headers: { "content-type": "application/json" },
          }
        );
      }

      // Process the form data
      const response = {
        success: true,
        message: "Form submitted successfully!",
        data: data,
        timestamp: new Date().toISOString(),
        processing: {
          name: data.name,
          email: data.email,
          age: data.age ? parseInt(data.age as string) : null,
        },
      };

      // Return success page
      const html = `
        <!DOCTYPE html>
        <html>
        <head>
          <title>Form Submitted</title>
          <style>
            body {
              font-family: Arial;
              display: flex;
              justify-content: center;
              align-items: center;
              height: 100vh;
              margin: 0;
              background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            }
            .container {
              background: white;
              padding: 40px;
              border-radius: 8px;
              text-align: center;
              box-shadow: 0 10px 40px rgba(0,0,0,0.2);
              max-width: 500px;
            }
            h1 {
              color: #667eea;
              margin-bottom: 20px;
            }
            .success-icon {
              font-size: 4em;
              margin-bottom: 20px;
            }
            pre {
              background: #f5f5f5;
              padding: 20px;
              border-radius: 4px;
              text-align: left;
              overflow-x: auto;
            }
            a {
              display: inline-block;
              margin-top: 20px;
              padding: 12px 30px;
              background: #667eea;
              color: white;
              text-decoration: none;
              border-radius: 4px;
            }
            a:hover {
              background: #764ba2;
            }
          </style>
        </head>
        <body>
          <div class="container">
            <div class="success-icon">✅</div>
            <h1>Form Submitted!</h1>
            <p>Thank you for submitting the form.</p>
            <h3>Received Data:</h3>
            <pre>${JSON.stringify(response, null, 2)}</pre>
            <a href="/">Submit Another</a>
          </div>
        </body>
        </html>
      `;

      return new Response(html, {
        headers: { "content-type": "text/html; charset=utf-8" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Failed to process form: " + (error as Error).message,
        }),
        {
          status: 500,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  return new Response("Not found", { status: 404 });
});
