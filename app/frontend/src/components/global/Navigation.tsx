import { useState } from "react";
import UpdaterBanner from "@ds/UpdaterBanner";
import PinkButton from "@ds/PinkButton";
import TransparentButton from "@ds/TransparentButton";

const buttons = [
	{ icon: "add", text: "New", href: "/chat", label: "", id: "new-idea" },
	{ icon: "home", text: "Dashboard", href: "/dashboard", label: "", id: "dashboard" },
	{ icon: "person", text: "My Profile", href: "/profile", label: "", id: "profile" }
];

function Navigation() {
	return (
		<>
			<UpdaterBanner />
			<NavigationInner />
		</>
	);
}

function NavigationInner() {
	const [isSidebarOpen, setIsSidebarOpen] = useState(false);

	const navButtons = buttons;

	const handleOpenSidebar = () => {
		setIsSidebarOpen(true);
	};

	const handleCloseSidebar = () => {
		setIsSidebarOpen(false);
	};

	const handleNavigate = (href: string) => {
		window.location.assign(href);
	};

	return (
		<div>
			{/* Open Sidebar Button */}
			<TransparentButton
				icon="menu"
				onClick={handleOpenSidebar}
				aria-label="Open Sidebar"
				classes="mt-2 ml-3 sm:hidden"
			/>

			{/* Sidebar */}
			<aside
				className={`top-0 left-0 z-40 w-64 h-full transition-transform sm:translate-x-0 fixed p-4 sm:pr-0 bg-stone-50
					${isSidebarOpen ? "" : "sm:sticky -translate-x-full"} `}
				aria-label="Sidebar"
			>
				<a href="/" className="flex justify-center items-center mt-2 mb-6 sm:mb-8">
					<img src="/logo.svg" className="h-8 mr-3 sm:h-12" alt="Brainstory Logo" />
				</a>
				<TransparentButton
					icon="close"
					onClick={handleCloseSidebar}
					aria-label="Close Sidebar"
					classes="absolute top-2 right-2 sm:hidden sm:block"
				/>
				<ul className="space-y-2 font-medium">
					{navButtons.map((button) => {
						if (button.id === "new-idea") {
							return (
								<li key="new-idea-pink" className="mx-5">
									<PinkButton
										icon={button.icon}
										iconClasses="mr-0.5"
										full
										role="link"
										onClick={() => {
											handleNavigate(button.href);
										}}
									>
										{button.text}
									</PinkButton>
									{button.id === "new-idea" && (
										<div className="text-center mt-2">
											<a
												className="text-xs font-normal text-stone-500 underline hover:text-slate-700 underline-offset-2 hover:no-underline"
												href="/get-started"
											>
												Guide me
											</a>
										</div>
									)}
									<hr className="w-48 h-[1px] mx-auto mt-4 mb-6 bg-stone-200 border-0 rounded" />
								</li>
							);
						}

						return (
							<li key={button.id}>
								<TransparentButton
									id={button.id}
									icon={button.icon}
									role="link"
									onClick={() => {
										handleNavigate(button.href);
									}}
									left
									full
								>
									{button.text}
								</TransparentButton>
							</li>
						);
					})}
				</ul>
			</aside>
		</div>
	);
}

export default Navigation;
