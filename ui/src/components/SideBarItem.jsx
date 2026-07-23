import { A } from '@solidjs/router'
import ListItem from '@suid/material/ListItem'
import ListItemButton from '@suid/material/ListItemButton'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import { children } from 'solid-js'

/**
 * @typedef {Object} SideBarItemProps
 * @property {string} text
 * @property {boolean} isFull
 * @property {string} link
 * @property {import("solid-js").JSXElement[]} children
 */

/**
 *
 * @param {SideBarItemProps} props
 */
const SideBarItem = (props) => {
	const c = children(() => props.children)

	return (
		<ListItem key={props.text} disablePadding sx={{ display: 'block', mb: 0.5 }}>
			<A href={props.link}>
				<ListItemButton
					sx={{
						minHeight: 48,
						justifyContent: props.isFull ? 'initial' : 'center',
						px: 2,
						mx: 0.5,
						borderRadius: 2.5,
						transition: 'background-color 0.18s ease, transform 0.18s ease',
						'&:hover': {
							bgcolor: 'rgba(20, 99, 92, 0.08)',
							transform: 'translateX(2px)',
						},
					}}
				>
					<ListItemIcon
						sx={{
							minWidth: 0,
							mr: props.isFull ? 2 : 'auto',
							justifyContent: 'center',
							color: 'primary.main',
						}}
					>
						{c()}
					</ListItemIcon>
					<ListItemText
						primary={props.text}
						sx={{
							display: props.isFull ? 'border-box' : 'none',
							'& .MuiTypography-root': {
								fontWeight: 600,
								fontSize: '0.95rem',
							},
						}}
					/>
				</ListItemButton>
			</A>
		</ListItem>
	)
}

export default SideBarItem
