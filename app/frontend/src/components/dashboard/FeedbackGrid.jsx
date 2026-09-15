import DailyStreakSection from "./DailyStreakSection.jsx";

// Share notifications were part of the original web app; on desktop the
// dashboard's feedback surface is just the streak section.
export default function FeedbackGrid() {
	return (
		<div className="mb-6 rounded-lg bg-pink-100 selection:bg-pink-300 selection:text-white mx-auto">
			<DailyStreakSection />
		</div>
	);
}
