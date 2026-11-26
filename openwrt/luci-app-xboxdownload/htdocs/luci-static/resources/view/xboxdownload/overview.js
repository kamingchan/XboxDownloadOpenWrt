'use strict';
'require view';
'require uci';
'require poll';
'require xboxdownload as xb';

function metrics(items) {
	return E('dl', { 'class': 'xbox-metrics' }, items.map(function(pair) {
		return E('div', {}, [ E('dt', {}, xb.text(pair[0])), E('dd', {}, xb.text(pair[1])) ]);
	}));
}

function section(title, content) { return E('section', {}, [ E('h3', {}, xb.text(title)), content ]); }

var names = {
	dns_redirect_a: _('Download DNS (A)'), dns_redirect_aaaa: _('Download DNS (AAAA)'),
	dns_srv_block: _('SRV fallback suppression'), http_redirect: _('Xbox content redirect'),
	http_dlassets_redirect: _('Legacy content redirect'),
	dnsmasq_redirect_a: _('Router DNS redirect (dnsmasq port 53)'),
	http_alias_redirect: _('LAN alias HTTP redirect (DNAT port 80)')
};

return view.extend({
	load: function() { return Promise.all([ xb.status(), xb.logs(50), uci.load('xboxdownload') ]); },
	render: function(data) {
		var summary = E('div'), groups = E('div'), scheduler = E('div'), logs = E('div');
		var checks = E('div', { 'aria-live': 'polite' }), problem = E('p', { role: 'status' });
		var actions = E('div', { 'class': 'xbox-actions' });
		var writable = !!L.hasViewPermission(), sequence = 0;
		function update(s, log) {
			summary.replaceChildren(metrics([
				[ _('Service'), s.running ? _('Running') : _('Stopped') ], [ _('Version'), s.version ],
				[ _('Uptime'), xb.uptime(s.uptime_secs) ], [ _('DNS listener'), s.dns_listen ],
				[ _('HTTP listener'), s.http_listen ], [ _('Redirect address'), s.redirect_ip ],
				[ _('DNS queries'), s.dns_queries_total ], [ _('Blocked SRV queries'), s.srv_blocked_total ],
				[ _('Filtered AAAA queries'), s.aaaa_filtered_total ], [ _('HTTP redirects'), s.http_redirects_total ]
			]));
			var buttons = s.running ? [ [ 'restart', _('Restart') ], [ 'stop', _('Stop') ] ] : [ [ 'start', _('Start') ] ];
			var disabled = uci.get('xboxdownload', 'main', 'enabled') !== '1';
			actions.replaceChildren.apply(actions, buttons.map(function(pair) {
				return xb.button(pair[1], function() { return xb.service_action(pair[0]).then(refresh); }, '', !writable || pair[0] === 'start' && disabled);
			}));
			if (!s.running && disabled) actions.append(xb.text(_('Enable the service in Settings to start it.')));
			groups.replaceChildren(xb.table([ _('Group'), _('Enabled'), _('Address family'), 'IPv4', 'IPv6', _('Last speed'), _('Last tested') ],
				Object.keys(xb.groups).map(function(id) {
					var g = (s.groups || {})[id] || uci.get('xboxdownload', id) || {};
					return [ xb.groups[id], g.enabled === true || g.enabled === '1' ? _('Yes') : _('No'), xb.families[g.ip_family],
						g.selected_ipv4, g.selected_ipv6, xb.speed(g.last_tested_ts ? g.last_speed_mbps : null), xb.time(g.last_tested_ts) ];
				}), ''));
			var a = s.scheduler || {}, state = { switched: _('Switched'), kept: _('Kept current node'),
				deferred: _('Deferred'), warning: _('Completed with warnings'), failed: _('Failed'), idle: _('Idle'), running: _('Running') };
			var enabled = a.auto_enabled == null ? uci.get('xboxdownload', 'speedtest', 'auto_enabled') === '1' : a.auto_enabled;
			scheduler.replaceChildren(metrics([ [ _('Automatic tests'), enabled ? _('Enabled') : _('Disabled') ],
				[ _('Last run'), xb.time(a.last_run_ts) ], [ _('Last result'), state[a.last_run_status] || a.last_run_status ],
				[ _('Details'), a.last_run_message ], [ _('Next run'), xb.time(a.next_run_ts) ] ]));
			logs.replaceChildren(xb.table([ _('Time'), _('Client IP'), _('Request host'), _('Path'), _('302 target') ],
				(log.entries || []).map(function(e) { return [ xb.time(e.ts), e.client_ip, e.host, e.path, e.target ]; }),
				_('No redirects recorded.')));
		}
		function refresh() {
			var seq = ++sequence;
			return Promise.all([ xb.status(), xb.logs(50) ]).then(function(v) {
				if (seq !== sequence) return;
				update(v[0], v[1]); problem.textContent = '';
			}).catch(function(e) { if (seq === sequence) problem.textContent = e.message; });
		}
		var checkButton = xb.button(_('Run end-to-end check'), function() {
			checks.textContent = _('Checking DNS and HTTP…');
			return xb.self_check().then(function(result) {
				checks.replaceChildren(xb.table([ _('Check'), _('Result'), _('Details') ], (result.checks || []).map(function(c) {
					return [ names[c.name] || c.name, c.passed ? _('Passed') : _('Failed'), c.detail ];
				}), _('No checks returned.')));
				return refresh();
			}).catch(function(e) { checks.textContent = e.message; });
		});
		update(data[0], data[1]);
		poll.add(refresh, 5);
		return E('div', { 'class': 'xbox-page' }, [ xb.stylesheet(), E('h2', {}, _('Xbox Download')), problem,
			section(_('Service status'), E('div', {}, [ actions, summary ])), section(_('CDN groups'), groups),
			section(_('Automatic speed tests'), scheduler), section(_('End-to-end check'), E('div', {}, [ checkButton, checks ])),
			section(_('Recent HTTP 302 redirects'), logs) ]);
	},
	handleSave: null, handleSaveApply: null, handleReset: null
});
