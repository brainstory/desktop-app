// One smoke test, no model needed: open the app, land on the dashboard,
// start a new idea, switch the recorder to text input, type a sentence,
// send it and assert the message appears in the transcript. The LLM is
// NOT driven - a missing model must not fail this test.
describe("brainstory smoke", () => {
	it("lands on the dashboard, types and sends a first message", async () => {
		// the window can sit on about:blank or the intro redirect for a
		// long while on a cold debug build: wait for a real page first
		await browser.waitUntil(
			async () => {
				const url = await browser.getUrl();
				return url !== "" && !url.startsWith("about:");
			},
			{ timeout: 120_000, timeoutMsg: "the webview never loaded a page" }
		);

		// first run redirects "/" to the get-started guide; the smoke
		// test drives the dashboard directly (same BaseLayout, same nav)
		const origin = new URL(await browser.getUrl()).origin;
		await browser.url(`${origin}/dashboard`);

		const newIdea = await $('[data-testid="new-idea-button"]');
		await newIdea.waitForDisplayed({
			timeout: 90_000,
			timeoutMsg: "the navigation (New button) never appeared on the dashboard"
		});
		await newIdea.click();

		// the recorder starts in whatever mode the profile dictates: on a
		// fresh profile the mic is blocked (no speech model yet), which
		// forces text input on. Only toggle when the composer is absent.
		const composer = await $('[data-testid="composer"]');
		try {
			await composer.waitForDisplayed({ timeout: 10_000 });
		} catch {
			const textMode = await $('[data-testid="text-mode-toggle"]');
			await textMode.waitForDisplayed({ timeout: 60_000 });
			await textMode.click();
			await composer.waitForDisplayed({ timeout: 30_000 });
		}
		const sentence = "I want to organize my garage workshop this weekend.";
		await composer.setValue(sentence);

		await $('[data-testid="send-button"]').click();

		// the sent message is the user's latest answer, rendered in the
		// conversation column
		const transcript = await $('[data-testid="transcript"]');
		await browser.waitUntil(async () => (await transcript.getText()).includes(sentence), {
			timeout: 30_000,
			timeoutMsg: "the sent message never appeared in the transcript"
		});
	});
});
