import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { connectSample, sampleSnapshot } from "../api/sample";
import { Doctor } from "./Doctor";
import { Logs } from "./Logs";
import { Sessions } from "./Sessions";
import { serve, withData } from "./storyData";

const sample = serve(sampleSnapshot);

export default { title: "Pages/Ops" } satisfies Meta;

const story = (render: () => React.JSX.Element, snapshot = sample, theme?: "light"): StoryObj => ({
    render,
    decorators: [withData(snapshot)],
    ...(theme && { globals: { theme } }),
});

export const SessionsDefault = story(() => <Sessions />);
export const SessionsLight = story(() => <Sessions />, sample, "light");
export const SessionsEmpty = story(() => <Sessions />, serve({ ...sampleSnapshot, sessions: [] }));

export const DoctorDefault = story(() => <Doctor />);
export const DoctorLight = story(() => <Doctor />, sample, "light");
// T977: a checkout with nothing to advise: every server installed, nothing measured.
export const DoctorProjectQuiet = story(
    () => <Doctor />,
    serve({
        ...sampleSnapshot,
        doctor: {
            ...sampleSnapshot.doctor!,
            project: {
                ...sampleSnapshot.doctor!.project!,
                languages: sampleSnapshot.doctor!.project!.languages.slice(0, 1).map((l) => ({
                    ...l,
                    saving: null,
                })),
                graph: null,
                plugins: [],
            },
        },
    }),
);
export const DoctorProbeFailed = story(
    () => <Doctor />,
    serve({ ...sampleSnapshot, doctor: null }),
);

// Select, read the diff and confirm against the mocked machine (T331.12).
export const DoctorFixFlow: StoryObj = {
    ...story(() => <Doctor />, connectSample),
    play: async ({ canvasElement }) => {
        const panel = within(await within(canvasElement).findByRole("region", { name: "fix" }));
        const project = await panel.findByRole("checkbox", { name: /stop\.sh in \/work\/app/ });
        await expect(project).not.toBeChecked();
        await expect(panel.getByLabelText("diff")).not.toHaveTextContent("/work/app");

        await userEvent.click(project);
        await expect(
            await panel.findByRole("button", { name: /Fix selected \(3\)/ }),
        ).toBeEnabled();
        await expect(panel.getByLabelText("diff")).toHaveTextContent("/work/app");

        await userEvent.click(panel.getByRole("button", { name: /Fix selected/ }));
        await expect(panel.getByText("Write 3 entries?")).toBeVisible();
        await userEvent.click(panel.getByRole("button", { name: "Confirm" }));
        await expect(await panel.findByRole("status")).toHaveTextContent("3 entries removed");
    },
};

export const LogsDefault = story(() => <Logs />);
export const LogsLight = story(() => <Logs />, sample, "light");
export const LogsEmpty = story(() => <Logs />, serve({ ...sampleSnapshot, logs: [] }));
