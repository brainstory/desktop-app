import RatingButtonGroup from "@components/global/RatingButtonGroup";

import type { SurveyItem } from "@helpers/api/forms";

interface EndSurveySectionProps {
	surveyItems: SurveyItem[];
	setSurveyItems: (items: SurveyItem[]) => void;
	starRange?: number;
}

export default function EndSurveySection({ surveyItems, setSurveyItems, starRange }: EndSurveySectionProps) {
	const renderSurveyFields = () => {
		return surveyItems.map((field: SurveyItem, labelIndex: number) => {
			const setRatingValue = (score: number) => {
				const updateSurveyInput = [...surveyItems];
				updateSurveyInput[labelIndex].value = score;
				setSurveyItems(updateSurveyInput);
			};
			const capitalizedLabel = field.id.charAt(0).toUpperCase() + field.id.slice(1);
			return (
				<RatingButtonGroup
					range={starRange}
					label={capitalizedLabel}
					key={capitalizedLabel}
					value={surveyItems[labelIndex].value}
					setRatingValue={setRatingValue}
				/>
			);
		});
	};

	return (
		<div className="my-5 py-3">
			<p className="mb-5 text-sm italic text-center">Rate how you feel</p>
			<div className="flex flex-col gap-5 max-w-[250px] mx-auto">{renderSurveyFields()}</div>
		</div>
	);
}
