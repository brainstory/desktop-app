import { useState, useEffect, useRef } from "react";
import UpdaterBanner from "@ds/UpdaterBanner";
import PinkButton from "@ds/PinkButton";
import TransparentButton from "@ds/TransparentButton";

const buttons = [
	{ icon: "add", text: "New", href: "/chat", id: "new-idea" },
	{ icon: "home", text: "Dashboard", href: "/dashboard", id: "dashboard" },
	{ icon: "settings-outline", text: "Settings", href: "/profile", id: "profile" }
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
	const openButtonRef = useRef<HTMLButtonElement | null>(null);
	const closeButtonRef = useRef<HTMLButtonElement | null>(null);

	const handleOpenSidebar = () => {
		setIsSidebarOpen(true);
	};

	const handleCloseSidebar = () => {
		setIsSidebarOpen(false);
	};

	// Mobile drawer behavior: Escape closes, focus starts inside the
	// drawer and returns to the hamburger button on close.
	useEffect(() => {
		if (!isSidebarOpen) return;

		closeButtonRef.current?.focus();

		const openButton = openButtonRef.current;
		const onKeyDown = (event: globalThis.KeyboardEvent): void => {
			if (event.key === "Escape") {
				setIsSidebarOpen(false);
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => {
			window.removeEventListener("keydown", onKeyDown);
			openButton?.focus();
		};
	}, [isSidebarOpen]);

	return (
		<div>
			{/* Open Sidebar Button */}
			<TransparentButton
				ref={openButtonRef}
				icon="menu"
				onClick={handleOpenSidebar}
				aria-label="Open Sidebar"
				classes="mt-2 ml-3 sm:hidden"
			/>

			{/* Mobile scrim behind the open drawer */}
			{isSidebarOpen && (
				<div
					className="fixed inset-0 z-30 bg-black opacity-40 sm:hidden"
					aria-hidden="true"
					onClick={handleCloseSidebar}
				/>
			)}

			{/* Sidebar */}
			<aside
				className={`top-0 left-0 z-40 w-64 h-full transition-transform sm:translate-x-0 fixed p-4 sm:pr-0 bg-stone-50 sm:sticky
					${isSidebarOpen ? "" : "-translate-x-full"} `}
				aria-label="Sidebar"
				// inert when closed: the off-screen drawer's links must not
				// be focusable or clickable while hidden
				inert={!isSidebarOpen ? true : undefined}
			>
				<a href="/" className="flex justify-center items-center mt-2 mb-6 sm:mb-8">
					<img src="/logo.svg" className="h-8 mr-3 sm:h-12" alt="Brainstory Logo" />
				</a>
				<TransparentButton
					ref={closeButtonRef}
					icon="close"
					onClick={handleCloseSidebar}
					aria-label="Close Sidebar"
					classes="absolute top-2 right-2 sm:hidden"
				/>
				<ul className="space-y-2 font-medium">
					{navButtons.map((button) => {
						if (button.id === "new-idea") {
							return (
								<li key="new-idea-pink" className="mx-5">
									{/* Real anchors: cmd-click / open-in-new-tab / screen
									    readers keep working */}
									<PinkButton
										icon={button.icon}
										iconClasses="mr-0.5"
										full
										href={button.href}
									>
										{button.text}
									</PinkButton>
									<div className="text-center mt-2">
										<a
											className="text-xs font-normal text-stone-500 underline hover:text-slate-700 underline-offset-2 hover:no-underline"
											href="/get-started"
										>
											Guide me
										</a>
									</div>
									<hr className="w-48 h-[1px] mx-auto mt-4 mb-6 bg-stone-200 border-0 rounded" />
								</li>
							);
						}

						const isActive =
							typeof window !== "undefined" &&
							window.location.pathname === button.href;
						return (
							<li key={button.id}>
								<TransparentButton
									id={button.id}
									icon={button.icon}
									href={button.href}
									left
									full
									aria-current={isActive ? "page" : undefined}
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
