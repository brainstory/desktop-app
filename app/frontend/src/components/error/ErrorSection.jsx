export default function ErrorSection({ title, paragraphs = [] }) {
	return (
		<div className="mx-auto w-full px-6 md:px-24 max-w-4xl py-12 scroll-mt-12">
			<p className="text-black font-bold lg:text-5xl text-4xl tracking-tight">{title}</p>
			<div className="text-stone-500 lg:text-xl text-base flex flex-col gap-3 mt-6">
				{paragraphs.map((pText, i) => {
					return <p key={i}>{pText}</p>;
				})}
				<p>If you keep seeing this page, try restarting the app.</p>
			</div>
		</div>
	);
}
