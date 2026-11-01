// One smoke test, no model needed: launch on the dashboard, start a new
// idea, switch the recorder to text input, type a sentence, send it and
// assert the message appears in the transcript. The LLM is NOT driven -
// a missing model must not fail this test.
describe("brainstory smoke", () => {
	it("lands on the dashboard, types and sends a first message", async () => {
		// the dashboard is the first-run landing page
		const newIdea = await $('[data-testid="new-idea-button"]');
		await newIdea.waitForDisplayed({ timeout: 60_000 });
		await newIdea.click();

		// the chat page: switch the recorder from the mic to typing
		const textMode = await $('[data-testid="text-mode-toggle"]');
		await textMode.waitForDisplayed({ timeout: 60_000 });
		await textMode.click();

		const composer = await $('[data-testid="composer"]');
		await composer.waitForDisplayed({ timeout: 30_000 });
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
